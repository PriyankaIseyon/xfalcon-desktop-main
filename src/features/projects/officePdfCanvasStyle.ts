const layoutAttribute = "data-office-pdf-layout";
const layoutRules = new WeakMap<HTMLCanvasElement, { sheet: CSSStyleSheet; selector: string }>();
let nextLayoutId = 0;

function sameOriginStylesheet(): CSSStyleSheet {
  const sheet = Array.from(document.styleSheets).find((candidate) => {
    if (!candidate.href || candidate.disabled) return false;
    try {
      return new URL(candidate.href, document.baseURI).origin === window.location.origin;
    } catch {
      return false;
    }
  });
  if (!sheet) throw new Error("PDF page layout is unavailable under the current security policy.");
  return sheet;
}

function removeRule(sheet: CSSStyleSheet, selector: string): void {
  for (let index = sheet.cssRules.length - 1; index >= 0; index -= 1) {
    const rule = sheet.cssRules[index];
    if ("selectorText" in rule && rule.selectorText === selector) {
      sheet.deleteRule(index);
      return;
    }
  }
}

export function clearOfficePdfCanvasSize(canvas: HTMLCanvasElement): void {
  const layout = layoutRules.get(canvas);
  if (layout) {
    removeRule(layout.sheet, layout.selector);
    layoutRules.delete(canvas);
  }
  canvas.removeAttribute(layoutAttribute);
}

export function setOfficePdfCanvasSize(canvas: HTMLCanvasElement, width: number, height: number): void {
  clearOfficePdfCanvasSize(canvas);
  if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) {
    throw new Error("This PDF page is too large to preview.");
  }

  const sheet = sameOriginStylesheet();
  const selector = `canvas[${layoutAttribute}="${++nextLayoutId}"]`;
  sheet.insertRule(`${selector}{width:${width}px;height:${height}px}`, sheet.cssRules.length);
  canvas.setAttribute(layoutAttribute, String(nextLayoutId));
  layoutRules.set(canvas, { sheet, selector });
}
