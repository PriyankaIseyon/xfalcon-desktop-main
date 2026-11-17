/**
 * The scale a PDF page is drawn at. Kept apart from `officePdfDocument.ts` so
 * the viewer can size a page without loading pdf.js.
 *
 * "Fit width" follows the room the window actually has: the page grows until it
 * fills the container, on a large display as on a small one. The only limits
 * are the ones the canvas imposes: no side of the drawn page may pass the
 * canvas edge limit (the renderer also lowers its pixel density for a very
 * large page, so the total pixel budget holds too).
 */
export const MIN_OFFICE_PDF_ZOOM = 0.5;
/** Far above any window: a 5,000-pixel-wide container shows a 720-point slide at about 7x. */
export const MAX_OFFICE_PDF_ZOOM = 8;
export const MAX_OFFICE_PDF_CANVAS_EDGE = 32_768;

/** `scale`, lowered if the drawn page would pass the canvas edge limit. */
export function limitScaleToCanvas(scale: number, pageWidth: number, pageHeight: number): number {
  const longest = Math.max(pageWidth, pageHeight);
  if (!(longest > 0)) return scale;
  return Math.min(scale, MAX_OFFICE_PDF_CANVAS_EDGE / longest);
}

/** The scale at which the page is as wide as the container, within what can be drawn. */
export function fitWidthScale(containerWidth: number, pageWidth: number, pageHeight: number): number {
  if (!(containerWidth > 0) || !(pageWidth > 0)) return 1;
  const fit = Math.min(MAX_OFFICE_PDF_ZOOM, containerWidth / pageWidth);
  return Math.max(0.01, limitScaleToCanvas(fit, pageWidth, pageHeight));
}

/**
 * The scale at which the whole page fits the container, both ways, so neither
 * scroll bar is needed. It grows a small page and shrinks a large one. Until
 * the height has been measured it falls back to fitting the width.
 */
export function fitPageScale(
  containerWidth: number,
  containerHeight: number,
  pageWidth: number,
  pageHeight: number,
): number {
  if (!(containerHeight > 0) || !(pageHeight > 0)) return fitWidthScale(containerWidth, pageWidth, pageHeight);
  if (!(containerWidth > 0) || !(pageWidth > 0)) return 1;
  const fit = Math.min(MAX_OFFICE_PDF_ZOOM, containerWidth / pageWidth, containerHeight / pageHeight);
  return Math.max(0.01, limitScaleToCanvas(fit, pageWidth, pageHeight));
}

export function clampOfficePdfZoom(value: number): number {
  return Math.max(MIN_OFFICE_PDF_ZOOM, Math.min(MAX_OFFICE_PDF_ZOOM, value));
}
