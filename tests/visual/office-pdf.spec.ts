import { readFileSync } from "node:fs";
import { expect, test, type Page } from "@playwright/test";
import { installVisualNativeMock } from "./native-mock";

const pdfBytes = readFileSync(new URL("../fixtures/office/producer-preview.pdf", import.meta.url));
const pdfBase64 = pdfBytes.toString("base64");
const desktopDevCsp = JSON.parse(readFileSync(new URL("../../src-tauri/tauri.conf.json", import.meta.url), "utf8")).app
  .security.devCsp as string;

interface OfficeCspViolation {
  sourceFile: string;
  effectiveDirective: string;
  blockedURI: string;
}

type OfficeCspWindow = Window & { __officeCspViolations: OfficeCspViolation[] };

async function applyDesktopDevCsp(page: Page) {
  await page.route("http://127.0.0.1:1420/", async (route) => {
    const response = await route.fetch();
    await route.fulfill({ response, headers: { ...response.headers(), "content-security-policy": desktopDevCsp } });
  });
}

test("renders a local Office PDF without Promise.try, with navigation, zoom, and page text", async ({ page }) => {
  // Safari 18 does not provide Promise.try. Exercise both the page and real
  // worker against that API boundary, using Mozilla's compatibility build.
  await page.addInitScript(() => Reflect.deleteProperty(Promise, "try"));
  await page.route("**/pdfjs-assets/pdf.worker.mjs", async (route) => {
    const response = await route.fetch();
    await route.fulfill({ response, body: `Reflect.deleteProperty(Promise, "try");\n${await response.text()}` });
  });
  await page.addInitScript(() => {
    const observedWindow = window as unknown as OfficeCspWindow;
    observedWindow.__officeCspViolations = [];
    document.addEventListener("securitypolicyviolation", (event) => {
      observedWindow.__officeCspViolations.push({
        sourceFile: event.sourceFile,
        effectiveDirective: event.effectiveDirective,
        blockedURI: event.blockedURI,
      });
    });
  });
  const requests: string[] = [];
  const browserErrors: string[] = [];
  page.on("request", (request) => requests.push(request.url()));
  page.on("pageerror", (error) => browserErrors.push(error.message));
  await applyDesktopDevCsp(page);
  await installVisualNativeMock(page);
  await page.goto("/");

  await page.evaluate(async (base64) => {
    const bytes = Uint8Array.from(atob(base64), (character) => character.charCodeAt(0));
    const importModule = (url: string) => import(/* @vite-ignore */ url);
    const [{ default: React }, { default: ReactDOM }, { OfficePdfViewer }] = await Promise.all([
      importModule("/node_modules/.vite/deps/react.js"),
      importModule("/node_modules/.vite/deps/react-dom_client.js"),
      importModule("/src/features/projects/OfficePdfViewer.tsx"),
    ]);
    const mount = document.createElement("div");
    mount.id = "office-pdf-test-root";
    mount.className = "fixed inset-0 z-30 flex";
    document.body.append(mount);
    const root = ReactDOM.createRoot(mount);
    (window as Window & { __officePdfRoot?: typeof root }).__officePdfRoot = root;
    root.render(React.createElement(OfficePdfViewer, { data: bytes, name: "producer.pdf" }));
  }, pdfBase64);

  const mount = page.locator("#office-pdf-test-root");
  await expect(mount).toBeAttached();
  await expect(mount).toContainText("Page 1 of 2", { timeout: 20_000 });
  const canvas = page.getByTestId("office-pdf-canvas");
  await expect(canvas).toBeVisible();
  await expect.poll(() => canvas.evaluate((element) => (element as HTMLCanvasElement).width)).toBeGreaterThan(0);
  const nonWhitePixels = await canvas.evaluate((element) => {
    const canvasElement = element as HTMLCanvasElement;
    const context = canvasElement.getContext("2d");
    if (!context) return 0;
    const { data } = context.getImageData(0, 0, canvasElement.width, canvasElement.height);
    let count = 0;
    for (let index = 0; index < data.length; index += 4) {
      if (data[index] < 245 || data[index + 1] < 245 || data[index + 2] < 245) count += 1;
    }
    return count;
  });
  expect(nonWhitePixels).toBeGreaterThan(100);

  await page.getByRole("button", { name: "Next page" }).click();
  await expect(page.getByText("Page 2 of 2")).toBeVisible();
  await expect(canvas).toBeVisible();
  const zoomLabel = page.locator('span[aria-label^="Zoom "]');
  const zoomPercent = async () =>
    Number((await zoomLabel.getAttribute("aria-label"))?.match(/Zoom (\d+) percent/)?.[1]);
  const initialZoom = await zoomPercent();
  await page.getByRole("button", { name: "Zoom out" }).click();
  await expect.poll(zoomPercent).toBeLessThan(initialZoom);
  const zoomedOut = await zoomPercent();
  await page.getByRole("button", { name: "Zoom in" }).click();
  await expect.poll(zoomPercent).toBeGreaterThan(zoomedOut);
  await page.getByRole("button", { name: "Fit width" }).click();
  await expect(page.getByRole("button", { name: "Fit width" })).toHaveAttribute("aria-pressed", "true");
  const textDisclosure = page.locator("details");
  await expect(textDisclosure).not.toHaveAttribute("open", "");
  await page.getByText("Text on page 2").click();
  await expect(textDisclosure).toHaveAttribute("open", "");
  const text = page.locator("details p");
  await expect(text).not.toHaveText("Loading page text…");
  expect((await text.innerText()).trim().length).toBeGreaterThan(0);

  await page.setViewportSize({ width: 260, height: 800 });
  await expect
    .poll(async () => Number((await zoomLabel.getAttribute("aria-label"))?.match(/Zoom (\d+) percent/)?.[1] ?? 100))
    .toBeLessThan(50);
  const fittedWidth = await canvas.evaluate((element) => Number.parseFloat(getComputedStyle(element).width));
  const pageRegionWidth = await page
    .getByRole("region", { name: "PDF page" })
    .evaluate((element) => element.clientWidth);
  expect(fittedWidth).toBeLessThanOrEqual(pageRegionWidth);

  const remoteRequests = requests.filter((url) => new URL(url).origin !== new URL(page.url()).origin);
  expect(remoteRequests).toEqual([]);
  expect(requests.some((url) => url.endsWith("/pdfjs-assets/pdf.worker.mjs"))).toBe(true);
  // WebKit console locations omit the responsible script. The CSP event
  // identifies Vite's development-only stylesheet injection in both engines.
  const cspViolations = await page.evaluate(() => (window as unknown as OfficeCspWindow).__officeCspViolations);
  expect(
    cspViolations.filter(
      (violation) =>
        !(
          violation.sourceFile === new URL("/@vite/client", page.url()).href &&
          violation.effectiveDirective === "style-src-elem" &&
          violation.blockedURI === "inline"
        ),
    ),
  ).toEqual([]);
  expect(browserErrors.filter((message) => message !== "WebSocket closed without opened.")).toEqual([]);

  await page.evaluate(() => {
    (window as Window & { __officePdfRoot?: { unmount: () => void } }).__officePdfRoot?.unmount();
  });
  await expect
    .poll(() =>
      page.evaluate(() =>
        Array.from(document.styleSheets).reduce((count, sheet) => {
          try {
            return (
              count +
              Array.from(sheet.cssRules).filter(
                (rule) =>
                  rule.type === CSSRule.STYLE_RULE &&
                  (rule as CSSStyleRule).selectorText.includes("data-office-pdf-layout"),
              ).length
            );
          } catch {
            return count;
          }
        }, 0),
      ),
    )
    .toBe(0);
});

test("replaces a rendered PDF with a clear malformed-file error", async ({ page }) => {
  await applyDesktopDevCsp(page);
  await installVisualNativeMock(page);
  await page.goto("/");
  await page.evaluate(async (base64) => {
    const bytes = Uint8Array.from(atob(base64), (character) => character.charCodeAt(0));
    const importModule = (url: string) => import(/* @vite-ignore */ url);
    const [{ default: React }, { default: ReactDOM }, { OfficePdfViewer }] = await Promise.all([
      importModule("/node_modules/.vite/deps/react.js"),
      importModule("/node_modules/.vite/deps/react-dom_client.js"),
      importModule("/src/features/projects/OfficePdfViewer.tsx"),
    ]);
    const mount = document.createElement("div");
    mount.id = "office-pdf-test-root";
    mount.className = "fixed inset-0 z-30 flex";
    document.body.append(mount);
    const root = ReactDOM.createRoot(mount);
    const testWindow = window as Window & {
      __officePdfRoot?: typeof root;
      __officePdfReplace?: (replacement: Uint8Array) => void;
    };
    testWindow.__officePdfRoot = root;
    root.render(React.createElement(OfficePdfViewer, { data: bytes, name: "producer.pdf" }));
    testWindow.__officePdfReplace = (replacement) =>
      root.render(React.createElement(OfficePdfViewer, { data: replacement, name: "broken.pdf" }));
  }, pdfBase64);

  await expect(page.getByText("Page 1 of 2")).toBeVisible();
  await expect(page.getByTestId("office-pdf-canvas")).toBeVisible();
  await page.evaluate(() => {
    (window as Window & { __officePdfReplace?: (replacement: Uint8Array) => void }).__officePdfReplace?.(
      new TextEncoder().encode("not a PDF"),
    );
  });
  await expect(page.getByText("PDF preview unavailable")).toBeVisible();
  await expect(
    page.getByText("This PDF could not be opened. It may be damaged or use an unsupported feature."),
  ).toBeVisible();
  await expect(page.getByTestId("office-pdf-canvas")).toHaveCount(0);
});
