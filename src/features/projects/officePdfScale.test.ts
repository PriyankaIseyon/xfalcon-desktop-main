import { describe, expect, it } from "vitest";
import {
  MAX_OFFICE_PDF_CANVAS_EDGE,
  MAX_OFFICE_PDF_ZOOM,
  MIN_OFFICE_PDF_ZOOM,
  clampOfficePdfZoom,
  fitPageScale,
  fitWidthScale,
  limitScaleToCanvas,
} from "./officePdfScale";

// A 16:9 slide as PDF points, and an A4 page.
const SLIDE = { width: 720, height: 405 };
const A4 = { width: 595, height: 842 };

describe("fitWidthScale", () => {
  it("fills a large window instead of stopping at 200%", () => {
    // A 1,904-pixel window minus the viewer's padding.
    const scale = fitWidthScale(1872, SLIDE.width, SLIDE.height);
    expect(scale).toBeCloseTo(1872 / 720, 5);
    expect(scale).toBeGreaterThan(2);
    expect(SLIDE.width * scale).toBeCloseTo(1872, 3);
  });

  it("also fits a small window, down below 100%", () => {
    expect(fitWidthScale(480, SLIDE.width, SLIDE.height)).toBeCloseTo(480 / 720, 5);
  });

  it("follows the window up to a 4K-wide container", () => {
    const scale = fitWidthScale(3808, SLIDE.width, SLIDE.height);
    expect(SLIDE.width * scale).toBeCloseTo(3808, 3);
  });

  it("does not blow a tiny page up past the largest zoom", () => {
    expect(fitWidthScale(1872, 50, 50)).toBe(MAX_OFFICE_PDF_ZOOM);
  });

  it("never lets the drawn page pass the canvas edge limit", () => {
    // A very tall page: width alone would allow 8x, its height would not.
    const scale = fitWidthScale(1872, 200, 20_000);
    expect(20_000 * scale).toBeLessThanOrEqual(MAX_OFFICE_PDF_CANVAS_EDGE + 1e-6);
  });

  it("uses 100% until the window has been measured", () => {
    expect(fitWidthScale(0, A4.width, A4.height)).toBe(1);
    expect(fitWidthScale(Number.NaN, A4.width, A4.height)).toBe(1);
    expect(fitWidthScale(1000, 0, 0)).toBe(1);
  });
});

describe("fitPageScale", () => {
  it("scales a slide up until its height meets a short, wide window", () => {
    // Wide and short: the height decides, so the width is not used up.
    const scale = fitPageScale(1872, 800, SLIDE.width, SLIDE.height);
    expect(scale).toBeCloseTo(800 / 405, 5);
    expect(SLIDE.width * scale).toBeLessThan(1872);
  });

  it("scales a slide up until its width meets a tall, narrow window", () => {
    const scale = fitPageScale(900, 1200, SLIDE.width, SLIDE.height);
    expect(scale).toBeCloseTo(900 / 720, 5);
    expect(SLIDE.height * scale).toBeLessThan(1200);
  });

  it("scales a page down when it is larger than the window, so nothing scrolls", () => {
    const scale = fitPageScale(480, 300, SLIDE.width, SLIDE.height);
    expect(scale).toBeLessThan(1);
    expect(SLIDE.width * scale).toBeLessThanOrEqual(480 + 1e-6);
    expect(SLIDE.height * scale).toBeLessThanOrEqual(300 + 1e-6);
  });

  it("never draws a page past the container in either direction", () => {
    for (const [w, h] of [
      [1872, 800],
      [900, 1200],
      [480, 300],
      [3808, 1900],
    ]) {
      for (const page of [SLIDE, A4]) {
        const scale = fitPageScale(w, h, page.width, page.height);
        expect(page.width * scale).toBeLessThanOrEqual(w + 1e-6);
        expect(page.height * scale).toBeLessThanOrEqual(h + 1e-6);
      }
    }
  });

  it("does not blow a tiny page up past the largest zoom or the canvas edge", () => {
    expect(fitPageScale(1872, 900, 50, 50)).toBe(MAX_OFFICE_PDF_ZOOM);
    const tall = fitPageScale(1872, 900, 200, 20_000);
    expect(20_000 * tall).toBeLessThanOrEqual(900 + 1e-6);
  });

  it("falls back to fitting the width until the height has been measured", () => {
    expect(fitPageScale(1872, 0, SLIDE.width, SLIDE.height)).toBe(fitWidthScale(1872, SLIDE.width, SLIDE.height));
    expect(fitPageScale(0, 0, A4.width, A4.height)).toBe(1);
    expect(fitPageScale(1000, 600, 0, 0)).toBe(1);
  });
});

describe("limitScaleToCanvas and the zoom range", () => {
  it("leaves a scale that fits alone and lowers one that does not", () => {
    expect(limitScaleToCanvas(2, A4.width, A4.height)).toBe(2);
    const limited = limitScaleToCanvas(100, A4.width, A4.height);
    expect(A4.height * limited).toBeCloseTo(MAX_OFFICE_PDF_CANVAS_EDGE, 3);
  });

  it("keeps manual zoom inside its range", () => {
    expect(clampOfficePdfZoom(0.1)).toBe(MIN_OFFICE_PDF_ZOOM);
    expect(clampOfficePdfZoom(3)).toBe(3);
    expect(clampOfficePdfZoom(50)).toBe(MAX_OFFICE_PDF_ZOOM);
  });
});
