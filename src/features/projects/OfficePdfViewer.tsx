import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { ChevronLeft, ChevronRight, Minus, Plus } from "lucide-react";
import type { PDFDocumentLoadingTask, PDFDocumentProxy, RenderTask } from "pdfjs-dist";
import { Alert } from "../../ui/Alert";
import { Button } from "../../ui/Button";
import { Spinner } from "../../ui/Spinner";
import { clearOfficePdfCanvasSize } from "./officePdfCanvasStyle";
import {
  MAX_OFFICE_PDF_ZOOM,
  MIN_OFFICE_PDF_ZOOM,
  clampOfficePdfZoom,
  fitPageScale,
  fitWidthScale,
  limitScaleToCanvas,
} from "./officePdfScale";

export interface OfficePdfViewerProps {
  data: Uint8Array;
  name: string;
}

type LoadState =
  | { data: Uint8Array; status: "loading" }
  | { data: Uint8Array; status: "error"; message: string }
  | { data: Uint8Array; status: "ready"; task: PDFDocumentLoadingTask; document: PDFDocumentProxy };

/** `page` shows the whole page (no scroll bars), `width` fills the width, `null` is a manual zoom. */
type FitMode = "page" | "width" | null;

type RenderState = {
  document: PDFDocumentProxy;
  pageNumber: number;
  width: number;
  /** The container height, kept only while fitting the whole page; 0 otherwise. */
  height: number;
  devicePixelRatio: number;
  fit: FitMode;
  zoom: number;
  scale: number;
  status: "ready" | "error";
  message?: string;
};

type PageTextState = {
  document: PDFDocumentProxy;
  pageNumber: number;
  status: "loading" | "ready" | "error";
  text?: string;
  truncated?: boolean;
};

function errorMessage(cause: unknown): string {
  if (cause instanceof Error && cause.message === "This PDF is empty.") return cause.message;
  if (cause instanceof Error && cause.message === "PDFs larger than 64 MiB cannot be previewed.") return cause.message;
  if (cause instanceof Error && cause.message === "This PDF page is too large to preview.") return cause.message;
  if (cause instanceof Error && /password|encrypt/i.test(cause.name + cause.message)) {
    return "This PDF is encrypted. Encrypted PDFs cannot be previewed.";
  }
  return "This PDF could not be opened. It may be damaged or use an unsupported feature.";
}

export function OfficePdfViewer({ data, name }: OfficePdfViewerProps) {
  const [loadState, setLoadState] = useState<LoadState>({ data, status: "loading" });
  const [pageNumber, setPageNumber] = useState(1);
  const [zoom, setZoom] = useState(1);
  const [fit, setFit] = useState<FitMode>("page");
  const [containerWidth, setContainerWidth] = useState(0);
  const [containerHeight, setContainerHeight] = useState(0);
  const [devicePixelRatio, setDevicePixelRatio] = useState(1);
  const [renderState, setRenderState] = useState<RenderState | null>(null);
  const [textDisclosureOpen, setTextDisclosureOpen] = useState(false);
  const [pageTextState, setPageTextState] = useState<PageTextState | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const renderTaskRef = useRef<RenderTask | null>(null);
  const renderIdleRef = useRef<Promise<void>>(Promise.resolve());

  const cancelRenderAndWait = useCallback(async () => {
    renderTaskRef.current?.cancel();
    await renderIdleRef.current.catch(() => undefined);
  }, []);

  useEffect(() => {
    let active = true;
    let task: PDFDocumentLoadingTask | null = null;
    setLoadState({ data, status: "loading" });
    setRenderState(null);
    setTextDisclosureOpen(false);
    setPageTextState(null);

    void (async () => {
      try {
        const { loadOfficePdf } = await import("./officePdfDocument");
        if (!active) return;
        const loadTask = loadOfficePdf(data);
        task = loadTask;
        const document = await loadTask.promise;
        if (!active) return;
        setPageNumber(1);
        setLoadState({ data, status: "ready", task: loadTask, document });
      } catch (cause) {
        if (active) setLoadState({ data, status: "error", message: errorMessage(cause) });
      }
    })();

    return () => {
      active = false;
      void (async () => {
        await cancelRenderAndWait();
        try {
          await task?.destroy();
        } catch {
          // PDF.js may reject teardown after a failed load; no document remains mounted.
        }
      })();
    };
  }, [data, cancelRenderAndWait]);

  const currentLoadState: LoadState = useMemo(
    () => (loadState.data === data ? loadState : { data, status: "loading" }),
    [data, loadState],
  );

  useLayoutEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    let frame = 0;
    let previousWidth = -1;
    let previousHeight = -1;

    // Rounded down, so a page sized to the container never overflows it by a fraction.
    const whole = (size: number) => (Number.isFinite(size) ? Math.max(0, Math.floor(size)) : 0);
    const measure = (width: number, height: number) => {
      const nextWidth = whole(width);
      const nextHeight = whole(height);
      if (nextWidth === previousWidth && nextHeight === previousHeight) return;
      previousWidth = nextWidth;
      previousHeight = nextHeight;
      setContainerWidth(nextWidth);
      setContainerHeight(nextHeight);
    };
    const update = () => measure(container.clientWidth - 32, container.clientHeight - 32);
    const observer = new ResizeObserver(([entry]) => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => measure(entry.contentRect.width, entry.contentRect.height));
    });
    observer.observe(container);
    update();

    const updatePixelRatio = () => setDevicePixelRatio(window.devicePixelRatio || 1);
    window.addEventListener("resize", updatePixelRatio);
    updatePixelRatio();
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
      window.removeEventListener("resize", updatePixelRatio);
    };
  }, [currentLoadState.status]);
  const currentPage =
    currentLoadState.status === "ready" ? Math.min(pageNumber, currentLoadState.document.numPages) : 1;
  // Only fitting the whole page depends on the height, so a height change
  // (opening the text panel, say) leaves the other modes' pages alone.
  const layoutHeight = fit === "page" ? containerHeight : 0;

  useEffect(() => {
    if (currentLoadState.status !== "ready") return;
    const document = currentLoadState.document;
    const canvas = canvasRef.current;
    if (!canvas) return;
    let cancelled = false;
    let task: RenderTask | null = null;
    const key = {
      document,
      pageNumber: currentPage,
      width: containerWidth,
      height: layoutHeight,
      devicePixelRatio,
      fit,
      zoom,
    };

    const clearCanvas = () => {
      clearOfficePdfCanvasSize(canvas);
      canvas.width = 0;
      canvas.height = 0;
    };

    void (async () => {
      await renderIdleRef.current.catch(() => undefined);
      if (cancelled) return;
      clearCanvas();
      try {
        const page = await document.getPage(currentPage);
        if (cancelled) return;
        const baseViewport = page.getViewport({ scale: 1 });
        const scale =
          fit === "page"
            ? fitPageScale(containerWidth, layoutHeight, baseViewport.width, baseViewport.height)
            : fit === "width"
              ? fitWidthScale(containerWidth, baseViewport.width, baseViewport.height)
              : limitScaleToCanvas(zoom, baseViewport.width, baseViewport.height);
        const pdfModule = await import("./officePdfDocument");
        if (cancelled) return;
        task = pdfModule.renderOfficePdfPage(page, canvas, scale, devicePixelRatio);
        renderTaskRef.current = task;
        renderIdleRef.current = task.promise.then(
          () => undefined,
          () => undefined,
        );
        await task.promise;
        if (!cancelled) {
          setRenderState({ ...key, scale, status: "ready" });
        }
      } catch (cause) {
        if (!cancelled) {
          clearCanvas();
          setRenderState({ ...key, scale: 1, status: "error", message: errorMessage(cause) });
        }
      } finally {
        if (task && renderTaskRef.current === task) renderTaskRef.current = null;
      }
    })();

    return () => {
      cancelled = true;
      if (task) {
        task.cancel();
        renderIdleRef.current = task.promise.then(
          () => undefined,
          () => undefined,
        );
      }
      void renderIdleRef.current.then(() => clearOfficePdfCanvasSize(canvas));
    };
  }, [currentLoadState, currentPage, containerWidth, layoutHeight, devicePixelRatio, fit, zoom]);

  const textDocument = currentLoadState.status === "ready" ? currentLoadState.document : null;
  useEffect(() => {
    if (!textDisclosureOpen || !textDocument) return;
    let cancelled = false;
    let textRead: ReturnType<typeof import("./officePdfDocument").readOfficePdfPageText> | null = null;
    setPageTextState({ document: textDocument, pageNumber: currentPage, status: "loading" });

    void (async () => {
      try {
        const pdfModule = await import("./officePdfDocument");
        if (cancelled) return;
        const page = await textDocument.getPage(currentPage);
        if (cancelled) return;
        textRead = pdfModule.readOfficePdfPageText(page);
        const result = await textRead.promise;
        if (!cancelled) {
          setPageTextState({
            document: textDocument,
            pageNumber: currentPage,
            status: "ready",
            text: result.text,
            truncated: result.truncated,
          });
        }
      } catch {
        if (!cancelled) {
          setPageTextState({ document: textDocument, pageNumber: currentPage, status: "error" });
        }
      }
    })();

    return () => {
      cancelled = true;
      textRead?.cancel();
    };
  }, [textDisclosureOpen, textDocument, currentPage]);

  if (currentLoadState.status === "loading") {
    return (
      <div aria-busy="true" className="flex min-h-0 flex-1 items-center justify-center" data-pdf-state="loading">
        <Spinner label={`Loading ${name}`} />
      </div>
    );
  }

  if (currentLoadState.status === "error") {
    return (
      <div className="p-4" data-pdf-state="error">
        <Alert title="PDF preview unavailable" tone="danger">
          <p>{currentLoadState.message}</p>
        </Alert>
      </div>
    );
  }

  const pageCount = currentLoadState.document.numPages;
  const renderMatchesCurrent =
    renderState?.status === "ready" &&
    renderState.document === currentLoadState.document &&
    renderState.pageNumber === currentPage &&
    renderState.width === containerWidth &&
    renderState.height === layoutHeight &&
    renderState.devicePixelRatio === devicePixelRatio &&
    renderState.fit === fit &&
    renderState.zoom === zoom;
  const renderErrorMatchesCurrent =
    renderState?.status === "error" &&
    renderState.document === currentLoadState.document &&
    renderState.pageNumber === currentPage &&
    renderState.width === containerWidth &&
    renderState.height === layoutHeight &&
    renderState.devicePixelRatio === devicePixelRatio &&
    renderState.fit === fit &&
    renderState.zoom === zoom;
  const pageTextMatchesCurrent =
    pageTextState?.document === currentLoadState.document && pageTextState.pageNumber === currentPage;
  // While a page redraws, keep showing the scale it had, not the manual zoom
  // (100% by default), which would flash on every page turn in a fit mode.
  const displayedScale = renderMatchesCurrent
    ? renderState.scale
    : renderState?.status === "ready" && renderState.document === currentLoadState.document
      ? renderState.scale
      : zoom;

  const changeZoom = (factor: number) => {
    const currentScale = renderMatchesCurrent ? renderState.scale : zoom;
    setZoom(clampOfficePdfZoom(currentScale * factor));
    setFit(null);
  };

  return (
    <section aria-label={`${name} PDF preview`} className="flex min-h-0 min-w-0 flex-1 flex-col" data-pdf-state="ready">
      <div
        aria-label="PDF controls"
        className="flex min-h-10 flex-wrap items-center gap-2 border-b border-hairline px-3 py-2"
        role="toolbar"
      >
        <Button
          aria-label="Previous page"
          disabled={currentPage <= 1}
          onClick={() => setPageNumber((value) => Math.max(1, value - 1))}
          size="sm"
        >
          <ChevronLeft aria-hidden size={16} />
        </Button>
        <span aria-live="polite" className="type-caption min-w-24 text-center tabular-nums text-secondary">
          Page {currentPage} of {pageCount}
        </span>
        <Button
          aria-label="Next page"
          disabled={currentPage >= pageCount}
          onClick={() => setPageNumber((value) => Math.min(pageCount, value + 1))}
          size="sm"
        >
          <ChevronRight aria-hidden size={16} />
        </Button>
        <span aria-hidden className="mx-1 h-5 border-l border-default" />
        <Button
          aria-label="Zoom out"
          disabled={displayedScale <= MIN_OFFICE_PDF_ZOOM}
          onClick={() => changeZoom(0.8)}
          size="sm"
        >
          <Minus aria-hidden size={16} />
        </Button>
        <span
          aria-label={`Zoom ${Math.round(displayedScale * 100)} percent`}
          role="status"
          className="type-caption min-w-12 text-center tabular-nums text-secondary"
        >
          {Math.round(displayedScale * 100)}%
        </span>
        <Button
          aria-label="Zoom in"
          disabled={displayedScale >= MAX_OFFICE_PDF_ZOOM}
          onClick={() => changeZoom(1.25)}
          size="sm"
        >
          <Plus aria-hidden size={16} />
        </Button>
        <Button onClick={() => setFit("page")} selected={fit === "page"} size="sm">
          Fit page
        </Button>
        <Button onClick={() => setFit("width")} selected={fit === "width"} size="sm">
          Fit width
        </Button>
      </div>

      <div
        aria-label="PDF page"
        aria-busy={!renderMatchesCurrent && !renderErrorMatchesCurrent}
        // Fitting the whole page needs no scroll bar (and a bar would shrink the
        // room it was measured against), so the page is centred and nothing scrolls.
        className={`min-h-0 min-w-0 flex-1 bg-surface-alt p-4 ${fit === "page" ? "flex flex-col overflow-hidden" : "overflow-auto"}`}
        ref={containerRef}
        role="region"
        tabIndex={0}
      >
        {renderErrorMatchesCurrent ? (
          <Alert title="Page preview unavailable" tone="danger">
            <p>{renderState.message}</p>
          </Alert>
        ) : null}
        <div className={fit === "page" ? "m-auto w-fit" : "mx-auto w-fit"}>
          <canvas
            aria-label={`Page ${currentPage} of ${pageCount}`}
            className={`block bg-card shadow-e1 ${renderMatchesCurrent ? "visible" : "invisible"}`}
            data-testid="office-pdf-canvas"
            ref={canvasRef}
            role="img"
          />
          {!renderMatchesCurrent && !renderErrorMatchesCurrent ? (
            <Spinner className="mt-3" label="Rendering PDF page" />
          ) : null}
        </div>
      </div>

      <details
        className="border-t border-hairline px-3 py-2"
        onToggle={(event) => {
          const open = event.currentTarget.open;
          setTextDisclosureOpen(open);
          if (!open) setPageTextState(null);
        }}
      >
        <summary className="type-caption cursor-pointer text-secondary">Text on page {currentPage}</summary>
        <p className="type-caption max-h-40 overflow-auto whitespace-pre-wrap py-2 text-foreground">
          {!textDisclosureOpen
            ? "Open to extract selectable text from this page."
            : !pageTextMatchesCurrent || pageTextState.status === "loading"
              ? "Loading page text…"
              : pageTextState.status === "error"
                ? "Text could not be extracted from this page."
                : pageTextState.text || "No selectable text is available on this page."}
          {textDisclosureOpen && pageTextMatchesCurrent && pageTextState.status === "ready" && pageTextState.truncated
            ? "\n\nText preview truncated at the safe limit."
            : null}
        </p>
      </details>
    </section>
  );
}
