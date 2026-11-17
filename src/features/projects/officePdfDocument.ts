import {
  AnnotationMode,
  getDocument,
  GlobalWorkerOptions,
  type PDFDocumentLoadingTask,
  type PDFPageProxy,
} from "pdfjs-dist/legacy/build/pdf.mjs";
import { setOfficePdfCanvasSize } from "./officePdfCanvasStyle";
import { MAX_OFFICE_PDF_CANVAS_EDGE } from "./officePdfScale";

export const MAX_OFFICE_PDF_BYTES = 64 * 1024 * 1024;
export const MAX_OFFICE_PDF_PIXELS = 16_000_000;
export { MAX_OFFICE_PDF_CANVAS_EDGE };
export const MAX_OFFICE_PDF_TEXT_CODE_UNITS = 128 * 1024;
export const MAX_OFFICE_PDF_TEXT_ITEMS = 5_000;

export interface OfficePdfPageText {
  text: string;
  truncated: boolean;
}

export interface OfficePdfPageTextRead {
  promise: Promise<OfficePdfPageText>;
  cancel: () => void;
}

GlobalWorkerOptions.workerSrc = `${import.meta.env.BASE_URL}pdfjs-assets/pdf.worker.mjs`;

export function loadOfficePdf(data: Uint8Array): PDFDocumentLoadingTask {
  if (data.byteLength === 0) throw new Error("This PDF is empty.");
  if (data.byteLength > MAX_OFFICE_PDF_BYTES) throw new Error("PDFs larger than 64 MiB cannot be previewed.");

  return getDocument({
    data: data.slice(),
    cMapUrl: `${import.meta.env.BASE_URL}pdfjs-assets/cmaps/`,
    cMapPacked: true,
    standardFontDataUrl: `${import.meta.env.BASE_URL}pdfjs-assets/standard_fonts/`,
    useWorkerFetch: false,
    useWasm: false,
    useSystemFonts: false,
    disableFontFace: true,
    enableXfa: false,
    stopAtErrors: true,
    maxImageSize: MAX_OFFICE_PDF_PIXELS,
    canvasMaxAreaInBytes: 64 * 1024 * 1024,
  });
}

export function readOfficePdfPageText(page: PDFPageProxy): OfficePdfPageTextRead {
  const reader = page.streamTextContent({ includeMarkedContent: false }).getReader();
  let cancelled = false;
  const promise = (async (): Promise<OfficePdfPageText> => {
    const textParts: string[] = [];
    let textLength = 0;
    let itemCount = 0;
    let truncated = false;
    try {
      while (!cancelled) {
        const { done, value } = await reader.read();
        if (done || cancelled) break;

        for (let itemIndex = 0; itemIndex < value.items.length; itemIndex += 1) {
          if (itemCount >= MAX_OFFICE_PDF_TEXT_ITEMS) {
            truncated = true;
            break;
          }
          const item = value.items[itemIndex];
          itemCount += 1;
          if ("str" in item && item.str.length > 0) {
            const separatorLength = textLength > 0 ? 1 : 0;
            const remaining = MAX_OFFICE_PDF_TEXT_CODE_UNITS - textLength - separatorLength;
            if (remaining <= 0) {
              truncated = true;
              break;
            }
            let part = item.str.slice(0, remaining);
            if (part.length < item.str.length && part.length > 0) {
              const lastCodeUnit = part.charCodeAt(part.length - 1);
              const nextCodeUnit = item.str.charCodeAt(part.length);
              if (
                lastCodeUnit >= 0xd800 &&
                lastCodeUnit <= 0xdbff &&
                nextCodeUnit >= 0xdc00 &&
                nextCodeUnit <= 0xdfff
              ) {
                part = part.slice(0, -1);
              }
            }
            if (part.length > 0) {
              if (separatorLength > 0) {
                textParts.push(" ");
                textLength += separatorLength;
              }
              textParts.push(part);
            }
            textLength += part.length;
            if (part.length < item.str.length || textLength >= MAX_OFFICE_PDF_TEXT_CODE_UNITS) {
              truncated = true;
              break;
            }
          }
          if (itemCount >= MAX_OFFICE_PDF_TEXT_ITEMS) {
            truncated = true;
            break;
          }
        }
        if (truncated) break;
      }
      return { text: textParts.join(""), truncated };
    } finally {
      if (!cancelled) {
        await reader.cancel().catch(() => undefined);
      }
      reader.releaseLock();
    }
  })();

  return {
    promise,
    cancel: () => {
      if (cancelled) return;
      cancelled = true;
      void reader.cancel().catch(() => undefined);
    },
  };
}

export function renderOfficePdfPage(
  page: PDFPageProxy,
  canvas: HTMLCanvasElement,
  scale: number,
  devicePixelRatio: number,
) {
  const viewport = page.getViewport({ scale });
  if (
    !Number.isFinite(viewport.width) ||
    !Number.isFinite(viewport.height) ||
    viewport.width <= 0 ||
    viewport.height <= 0 ||
    viewport.width > MAX_OFFICE_PDF_CANVAS_EDGE ||
    viewport.height > MAX_OFFICE_PDF_CANVAS_EDGE
  ) {
    throw new Error("This PDF page is too large to preview.");
  }

  const safeRatio = Number.isFinite(devicePixelRatio) && devicePixelRatio > 0 ? devicePixelRatio : 1;
  const pixelRatio = Math.min(
    safeRatio,
    Math.sqrt(MAX_OFFICE_PDF_PIXELS / (viewport.width * viewport.height)),
    MAX_OFFICE_PDF_CANVAS_EDGE / viewport.width,
    MAX_OFFICE_PDF_CANVAS_EDGE / viewport.height,
  );
  const pixelWidth = Math.max(1, Math.floor(viewport.width * pixelRatio));
  const pixelHeight = Math.max(1, Math.floor(viewport.height * pixelRatio));
  if (
    !Number.isFinite(pixelWidth) ||
    !Number.isFinite(pixelHeight) ||
    pixelWidth * pixelHeight > MAX_OFFICE_PDF_PIXELS
  ) {
    throw new Error("This PDF page is too large to preview.");
  }

  canvas.width = pixelWidth;
  canvas.height = pixelHeight;
  setOfficePdfCanvasSize(canvas, viewport.width, viewport.height);

  return page.render({
    canvas,
    viewport,
    transform: pixelRatio === 1 ? undefined : [pixelRatio, 0, 0, pixelRatio, 0, 0],
    annotationMode: AnnotationMode.DISABLE,
  });
}
