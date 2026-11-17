import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { OfficePdfViewer } from "./OfficePdfViewer";
import { loadOfficePdf, readOfficePdfPageText, renderOfficePdfPage } from "./officePdfDocument";

vi.mock("./officePdfDocument", () => ({
  loadOfficePdf: vi.fn(),
  readOfficePdfPageText: vi.fn(),
  renderOfficePdfPage: vi.fn(),
}));

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function createPage(width = 600, height = 800) {
  return {
    getViewport: vi.fn(({ scale }: { scale: number }) => ({ width: width * scale, height: height * scale })),
  };
}

function createDocument(numPages = 2) {
  const pages = Array.from({ length: numPages }, (_, index) => createPage(600 - index * 100, 800));
  return {
    numPages,
    getPage: vi.fn(async (pageNumber: number) => pages[pageNumber - 1]),
  };
}

function loadingTask(document = createDocument()) {
  return {
    promise: Promise.resolve(document),
    destroy: vi.fn(async () => undefined),
  };
}

function renderTask() {
  const done = deferred<void>();
  const task = {
    promise: done.promise,
    cancel: vi.fn(() => done.reject(new Error("render canceled"))),
  };
  done.resolve();
  return task;
}

describe("OfficePdfViewer", () => {
  beforeEach(() => {
    vi.mocked(loadOfficePdf).mockReset();
    vi.mocked(readOfficePdfPageText).mockReset();
    vi.mocked(renderOfficePdfPage).mockReset();
    vi.mocked(readOfficePdfPageText).mockReturnValue({
      promise: Promise.resolve({ text: "Page text from the PDF.", truncated: false }),
      cancel: vi.fn(),
    });
    vi.mocked(renderOfficePdfPage).mockImplementation((_page, canvas) => {
      canvas.width = 600;
      canvas.height = 800;
      canvas.style.width = "600px";
      canvas.style.height = "800px";
      return renderTask() as never;
    });
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.clearAllMocks();
  });

  it("renders the current page, exposes selectable text, navigates and changes zoom", async () => {
    const document = createDocument(2);
    vi.mocked(loadOfficePdf).mockReturnValue(loadingTask(document) as never);
    render(<OfficePdfViewer data={new Uint8Array([1, 2, 3])} name="producer.pdf" />);

    const canvas = await screen.findByRole("img", { name: "Page 1 of 2" });
    await waitFor(() => expect(canvas).toBeVisible());
    expect(screen.getByText("Page 1 of 2")).toBeVisible();
    expect(document.getPage).toHaveBeenCalledWith(1);

    fireEvent.click(screen.getByRole("button", { name: "Next page" }));
    await screen.findByRole("img", { name: "Page 2 of 2" });
    expect(document.getPage).toHaveBeenCalledWith(2);
    expect(readOfficePdfPageText).not.toHaveBeenCalled();
    fireEvent.click(screen.getByText("Text on page 2"));
    expect(await screen.findByText("Page text from the PDF.")).toBeVisible();
    expect(readOfficePdfPageText).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    await waitFor(() => expect(screen.getByLabelText("Zoom 125 percent")).toBeVisible());
    fireEvent.click(screen.getByRole("button", { name: "Fit width" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Fit width" })).toHaveAttribute("aria-pressed", "true"),
    );
  });

  it("fits the whole page by default, with no scroll bars, and lets the user fit the width instead", async () => {
    // The page region's content box: 1,000 x 600 after the viewer's 16px padding.
    const widthSpy = vi.spyOn(Element.prototype, "clientWidth", "get").mockReturnValue(1032);
    const heightSpy = vi.spyOn(Element.prototype, "clientHeight", "get").mockReturnValue(632);
    try {
      vi.mocked(loadOfficePdf).mockReturnValue(loadingTask(createDocument(2)) as never);
      render(<OfficePdfViewer data={new Uint8Array([1, 2, 3])} name="deck.pdf" />);

      const region = await screen.findByRole("region", { name: "PDF page" });
      const scales = () => vi.mocked(renderOfficePdfPage).mock.calls.map((call) => call[2]);
      const fitPage = screen.getByRole("button", { name: "Fit page" });
      const fitWidth = screen.getByRole("button", { name: "Fit width" });
      expect(fitPage).toHaveAttribute("aria-pressed", "true");
      expect(fitWidth).toHaveAttribute("aria-pressed", "false");
      // A 600 x 800 page in 1,000 x 600: the height decides (0.75), not the width (1.67).
      await waitFor(() => expect(scales().at(-1)).toBeCloseTo(0.75, 5));
      expect(screen.getByLabelText("Zoom 75 percent")).toBeVisible();
      expect(region).toHaveClass("overflow-hidden");
      expect(region).not.toHaveClass("overflow-auto");

      fireEvent.click(fitWidth);
      await waitFor(() => expect(scales().at(-1)).toBeCloseTo(1000 / 600, 5));
      expect(fitWidth).toHaveAttribute("aria-pressed", "true");
      expect(region).toHaveClass("overflow-auto");

      fireEvent.click(fitPage);
      await waitFor(() => expect(scales().at(-1)).toBeCloseTo(0.75, 5));
      expect(region).toHaveClass("overflow-hidden");

      fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
      await waitFor(() => expect(scales().at(-1)).toBeCloseTo(0.9375, 5));
      expect(fitPage).toHaveAttribute("aria-pressed", "false");
      expect(region).toHaveClass("overflow-auto");
    } finally {
      widthSpy.mockRestore();
      heightSpy.mockRestore();
    }
  });

  it("clears the previous document and reports empty, oversized, and malformed inputs", async () => {
    const firstData = new Uint8Array([1, 2, 3]);
    const firstTask = loadingTask(createDocument());
    vi.mocked(loadOfficePdf)
      .mockReturnValueOnce(firstTask as never)
      .mockImplementationOnce(() => {
        throw new Error("This PDF is empty.");
      });
    const view = render(<OfficePdfViewer data={firstData} name="first.pdf" />);
    await screen.findByRole("img", { name: "Page 1 of 2" });

    view.rerender(<OfficePdfViewer data={new Uint8Array()} name="empty.pdf" />);
    expect(await screen.findByText("This PDF is empty.")).toBeVisible();
    expect(screen.queryByTestId("office-pdf-canvas")).not.toBeInTheDocument();
    expect(firstTask.destroy).toHaveBeenCalled();

    vi.mocked(loadOfficePdf).mockImplementationOnce(() => {
      throw new Error("PDFs larger than 64 MiB cannot be previewed.");
    });
    view.rerender(<OfficePdfViewer data={new Uint8Array([4, 5, 6])} name="large.pdf" />);
    expect(await screen.findByText("PDFs larger than 64 MiB cannot be previewed.")).toBeVisible();

    vi.mocked(loadOfficePdf).mockReturnValueOnce({
      promise: Promise.reject(new Error("Invalid PDF structure")),
      destroy: vi.fn(async () => undefined),
    } as never);
    view.rerender(<OfficePdfViewer data={new Uint8Array([7, 8, 9])} name="broken.pdf" />);
    expect(
      await screen.findByText("This PDF could not be opened. It may be damaged or use an unsupported feature."),
    ).toBeVisible();
  });

  it("waits for a canceled render before reusing the canvas and destroys the task on unmount", async () => {
    const document = createDocument(2);
    const task = loadingTask(document);
    vi.mocked(loadOfficePdf).mockReturnValue(task as never);
    const firstRender = deferred<void>();
    let renderCount = 0;
    vi.mocked(renderOfficePdfPage).mockImplementation((_page, canvas) => {
      renderCount += 1;
      canvas.width = 600;
      canvas.height = 800;
      const pending = renderCount === 1 ? firstRender : deferred<void>();
      return {
        promise: pending.promise,
        cancel: vi.fn(() => pending.reject(new Error("render canceled"))),
      } as never;
    });

    const view = render(<OfficePdfViewer data={new Uint8Array([1])} name="producer.pdf" />);
    await waitFor(() => expect(renderCount).toBe(1));
    fireEvent.click(screen.getByRole("button", { name: "Next page" }));
    await waitFor(() => expect(renderCount).toBe(2));
    expect(vi.mocked(renderOfficePdfPage).mock.results[0].value.cancel).toHaveBeenCalled();

    view.unmount();
    await waitFor(() => expect(task.destroy).toHaveBeenCalled());
    expect(task.destroy.mock.invocationCallOrder[0]).toBeGreaterThan(
      vi.mocked(renderOfficePdfPage).mock.results[1].value.cancel.mock.invocationCallOrder[0],
    );
  });

  it("keeps the canvas mounted after a page render error and recovers on navigation and resize", async () => {
    const document = createDocument(2);
    vi.mocked(loadOfficePdf).mockReturnValue(loadingTask(document) as never);
    const observers: Array<{ callback: ResizeObserverCallback; target: Element | null }> = [];
    vi.stubGlobal(
      "ResizeObserver",
      class {
        private readonly entry: (typeof observers)[number];
        constructor(callback: ResizeObserverCallback) {
          this.entry = { callback, target: null };
          observers.push(this.entry);
        }
        observe(target: Element) {
          this.entry.target = target;
        }
        disconnect() {
          this.entry.target = null;
        }
      },
    );
    let renderCount = 0;
    vi.mocked(renderOfficePdfPage).mockImplementation((_page, canvas) => {
      renderCount += 1;
      canvas.width = 600;
      canvas.height = 800;
      const result = renderCount === 1 ? Promise.reject(new Error("page render failed")) : Promise.resolve();
      return { promise: result, cancel: vi.fn() } as never;
    });

    render(<OfficePdfViewer data={new Uint8Array([1, 2, 3])} name="producer.pdf" />);
    const canvas = await screen.findByTestId("office-pdf-canvas");
    expect(await screen.findByText("Page preview unavailable")).toBeVisible();
    expect(canvas).toHaveClass("invisible");
    expect(canvas).toHaveAttribute("width", "0");

    fireEvent.click(screen.getByRole("button", { name: "Next page" }));
    await waitFor(() => expect(canvas).toBeVisible());
    expect(screen.getByText("Page 2 of 2")).toBeVisible();
    expect(renderCount).toBe(2);

    const pageRegion = screen.getByRole("region", { name: "PDF page" });
    for (const observer of observers) {
      if (observer.target === pageRegion) {
        observer.callback(
          [{ target: pageRegion, contentRect: { width: 480 } } as unknown as ResizeObserverEntry],
          {} as ResizeObserver,
        );
      }
    }
    await waitFor(() => expect(renderCount).toBe(3));
    expect(canvas).toBeVisible();
  });

  it("rejects invalid PDF input before rendering a page", async () => {
    vi.mocked(loadOfficePdf).mockReturnValue({
      promise: Promise.reject(Object.assign(new Error("password required"), { name: "PasswordException" })),
      destroy: vi.fn(async () => undefined),
    } as never);
    render(<OfficePdfViewer data={new Uint8Array([1])} name="encrypted.pdf" />);
    expect(await screen.findByText("This PDF is encrypted. Encrypted PDFs cannot be previewed.")).toBeVisible();
    expect(renderOfficePdfPage).not.toHaveBeenCalled();
  });

  it("keeps canvas rendering independent and cancels lazy text work on page change and disclosure close", async () => {
    const document = createDocument(2);
    vi.mocked(loadOfficePdf).mockReturnValue(loadingTask(document) as never);
    const reads: Array<{
      cancel: ReturnType<typeof vi.fn>;
      promise: ReturnType<typeof deferred<{ text: string; truncated: boolean }>>;
    }> = [];
    vi.mocked(readOfficePdfPageText).mockImplementation(() => {
      const pending = deferred<{ text: string; truncated: boolean }>();
      const cancel = vi.fn();
      reads.push({ promise: pending, cancel });
      return { promise: pending.promise, cancel };
    });

    render(<OfficePdfViewer data={new Uint8Array([1, 2, 3])} name="producer.pdf" />);
    const canvas = await screen.findByTestId("office-pdf-canvas");
    await waitFor(() => expect(canvas).toBeVisible());
    expect(readOfficePdfPageText).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText("Text on page 1"));
    await waitFor(() => expect(readOfficePdfPageText).toHaveBeenCalledTimes(1));
    expect(canvas).toBeVisible();
    expect(screen.getByText("Loading page text…")).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "Next page" }));
    await waitFor(() => expect(readOfficePdfPageText).toHaveBeenCalledTimes(2));
    expect(reads[0].cancel).toHaveBeenCalledTimes(1);
    expect(canvas).toBeVisible();
    expect(screen.getByText("Page 2 of 2")).toBeVisible();

    fireEvent.click(screen.getByText("Text on page 2"));
    await waitFor(() => expect(reads[1].cancel).toHaveBeenCalledTimes(1));
    expect(screen.queryByText("Page text from the PDF.")).not.toBeInTheDocument();
  });

  it("shows when the bounded selectable-text preview was truncated", async () => {
    vi.mocked(loadOfficePdf).mockReturnValue(loadingTask(createDocument(1)) as never);
    vi.mocked(readOfficePdfPageText).mockReturnValue({
      promise: Promise.resolve({ text: "Bounded page text", truncated: true }),
      cancel: vi.fn(),
    });
    render(<OfficePdfViewer data={new Uint8Array([1, 2, 3])} name="producer.pdf" />);

    fireEvent.click(await screen.findByText("Text on page 1"));
    expect(await screen.findByText(/Bounded page text/)).toBeVisible();
    expect(screen.getByText(/Text preview truncated at the safe limit/)).toBeVisible();
  });

  it("cancels page-text streams when the document changes or the viewer unmounts", async () => {
    const firstTask = loadingTask(createDocument(1));
    const secondTask = loadingTask(createDocument(1));
    vi.mocked(loadOfficePdf)
      .mockReturnValueOnce(firstTask as never)
      .mockReturnValueOnce(secondTask as never);
    const reads: Array<{
      cancel: ReturnType<typeof vi.fn>;
      promise: ReturnType<typeof deferred<{ text: string; truncated: boolean }>>;
    }> = [];
    vi.mocked(readOfficePdfPageText).mockImplementation(() => {
      const pending = deferred<{ text: string; truncated: boolean }>();
      const cancel = vi.fn();
      reads.push({ promise: pending, cancel });
      return { promise: pending.promise, cancel };
    });

    const view = render(<OfficePdfViewer data={new Uint8Array([1])} name="first.pdf" />);
    await screen.findByTestId("office-pdf-canvas");
    fireEvent.click(screen.getByText("Text on page 1"));
    await waitFor(() => expect(reads).toHaveLength(1));

    view.rerender(<OfficePdfViewer data={new Uint8Array([2])} name="second.pdf" />);
    await waitFor(() => expect(firstTask.destroy).toHaveBeenCalled());
    await screen.findByTestId("office-pdf-canvas");
    expect(reads[0].cancel).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByText("Text on page 1"));
    await waitFor(() => expect(reads).toHaveLength(2));
    view.unmount();
    expect(reads[1].cancel).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(secondTask.destroy).toHaveBeenCalled());
  });
});
