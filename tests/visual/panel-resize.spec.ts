import { expect, test } from "@playwright/test";
import { installDatasetReportsMock } from "./datasets-reports-native-mock";

for (const theme of ["light", "dark"] as const) {
  test(`panel resizes with pointer and keyboard in ${theme}`, async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 960 });
    await page.emulateMedia({ reducedMotion: "reduce", colorScheme: theme });
    await installDatasetReportsMock(page, { theme });
    await page.goto("/");
    await expect(page.getByRole("button", { name: "Settings", exact: true })).toBeVisible();
    await page.evaluate(async () => {
      const path = "/src/features/canvas/canvasStore.ts";
      const store = await import(/* @vite-ignore */ path);
      store.openCanvasArtifact({
        id: "resize-table",
        type: "table",
        title: "Sales",
        columns: ["Product", "Revenue"],
        rows: [
          ["A", 100],
          ["B", 250],
        ],
      });
    });
    const panel = page.getByRole("complementary", { name: "Artifact side panel" });
    const divider = page.getByRole("separator", { name: "Artifact side panel" });
    const shell = page.locator(".app-shell");
    await expect(divider).toBeVisible();
    await expect(divider.locator(".canvas-divider__grip")).toBeVisible();
    await expect(divider.locator(".canvas-divider__grip > span")).toHaveCount(3);
    // Two surfaces divided by single lines (the Claude desktop layout): no
    // gutter, no pane corners, one edge at each junction.
    await expect(shell).toHaveCSS("column-gap", "0px");
    for (const pane of [
      page.locator(".app-sidebar"),
      page.locator(".app-workspace__body"),
      page.locator("#canvas-artifact-panel"),
    ]) {
      await expect(pane).toHaveCSS("border-radius", "0px");
    }
    await expect(page.locator(".app-sidebar")).toHaveCSS("border-right-width", "1px");
    await expect(page.locator(".app-workspace__body")).toHaveCSS("border-right-width", "0px");
    await expect(page.locator("#canvas-artifact-panel")).toHaveCSS("border-left-width", "1px");
    // The title bar is the workspace's own top row: no border and flush with the
    // body. The artifact panel keeps a hairline under its header.
    for (const [headerSelector, bodySelector, rule] of [
      [".app-titlebar", ".app-workspace__body", "0px"],
      [".canvas-panel__header", "#canvas-artifact-panel", "1px"],
    ]) {
      const header = page.locator(headerSelector);
      const body = page.locator(bodySelector);
      await expect(header).toHaveCSS("border-radius", "0px");
      await expect(header).toHaveCSS("box-shadow", "none");
      await expect(header).toHaveCSS("border-bottom-width", rule);
      await expect(body).toHaveCSS("border-top-width", "0px");
      const headerBox = (await header.boundingBox())!;
      const bodyBox = (await body.boundingBox())!;
      expect(bodyBox.y - headerBox.y - headerBox.height).toBeCloseTo(0, 1);
      expect(await body.evaluate((node) => parseFloat(getComputedStyle(node).borderTopLeftRadius))).toBe(0);
    }
    const workspace = (await page.locator(".app-workspace").boundingBox())!;
    const initial = (await panel.boundingBox())!;
    const assertAligned = async () => {
      const chatHeader = (await page.locator(".app-titlebar").boundingBox())!;
      const panelHeader = (await page.locator(".canvas-panel__header").boundingBox())!;
      expect(panelHeader.y).toBeCloseTo(chatHeader.y, 1);
      expect(panelHeader.height).toBeCloseTo(chatHeader.height, 1);
      const chatBody = (await page.locator(".app-workspace__body").boundingBox())!;
      const panelBody = (await page.locator("#canvas-artifact-panel").boundingBox())!;
      expect(panelBody.y).toBeCloseTo(chatBody.y, 1);
      expect(panelBody.y + panelBody.height).toBeCloseTo(chatBody.y + chatBody.height, 1);
      const overflow = await panel.evaluate((node) =>
        [...node.querySelectorAll<HTMLElement>("*")]
          .filter((element) => element.tagName !== "IFRAME" && /auto|scroll/.test(getComputedStyle(element).overflowY))
          .filter((element) => element.scrollHeight > element.clientHeight + 1)
          .map((element) => element.className),
      );
      expect(overflow).toEqual([]);
    };
    await assertAligned();
    const brand = (await page.locator(".app-sidebar__brand").boundingBox())!;
    const titlebar = (await page.locator(".app-titlebar").boundingBox())!;
    expect(brand.y + brand.height / 2).toBeCloseTo(titlebar.y + titlebar.height / 2, 0);
    expect(initial.x - workspace.x - workspace.width).toBeCloseTo(0, 1);
    const handle = (await divider.boundingBox())!;
    await page.mouse.move(handle.x + handle.width / 2, 300);
    await page.mouse.down();
    await page.mouse.move(5, 300, { steps: 15 });
    await page.mouse.up();
    await expect(divider).toHaveAttribute("aria-valuenow", "70");
    await expect(shell).not.toHaveAttribute("data-canvas-resizing", "true");
    const maximum = (await panel.boundingBox())!;
    expect(maximum.width).toBeCloseTo(1440 * 0.7, 0);
    expect((await page.locator(".app-sidebar").boundingBox())!.width).toBeCloseTo(1440 * 0.15, 0);
    expect((await page.locator(".app-workspace").boundingBox())!.width).toBeCloseTo(1440 * 0.15, 0);
    await page.screenshot({ path: `/tmp/xf-panel-${theme}-maximum.png` });
    await divider.focus();
    await page.keyboard.press("Home");
    await expect(divider).toHaveAttribute("aria-valuenow", "20");
    await page.keyboard.press("Shift+ArrowLeft");
    await expect(divider).toHaveAttribute("aria-valuenow", "25");
    await page.keyboard.press("ArrowRight");
    await expect(divider).toHaveAttribute("aria-valuenow", "24");
    await page.keyboard.press("Enter");
    expect((await panel.boundingBox())!.width).toBeCloseTo(initial.width, 0);
    await page.getByRole("button", { name: "Expand panel", exact: true }).click();
    await expect(divider).toHaveAttribute("aria-valuenow", "70");
    await expect(page.locator(".app-workspace")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(divider).toHaveAttribute("aria-valuenow", "42");
    await divider.focus();
    await page.keyboard.press("ArrowLeft");
    await page.getByRole("button", { name: "Close side panel" }).click();
    await expect(panel).toHaveCount(0);
    await page.getByRole("button", { name: "Show side panel", exact: true }).click();
    await expect(divider).toHaveAttribute("aria-valuenow", "43");
    // Drag across the real document iframe without losing pointer capture.
    await page.route("http://xf-preview.localhost/**", (route) =>
      route.fulfill({ contentType: "text/html", body: "<button>Artifact action</button>" }),
    );
    await page.evaluate(async () => {
      const path = "/src/features/canvas/canvasStore.ts";
      const store = await import(/* @vite-ignore */ path);
      store.openCanvasArtifact({
        id: "resize-html",
        type: "html",
        kind: "html",
        title: "Document",
        documentId: "old",
        content: "<button>Artifact action</button>",
      });
    });
    const frame = panel.locator("iframe");
    await expect(frame).toBeVisible();
    await assertAligned();
    await page.screenshot({ path: `/tmp/xf-aligned-shell-${theme}.png` });
    const documentHandle = (await divider.boundingBox())!;
    await page.mouse.move(documentHandle.x + documentHandle.width / 2, 320);
    await page.mouse.down();
    await page.mouse.move(1430, 320, { steps: 12 });
    await page.mouse.up();
    await expect(divider).toHaveAttribute("aria-valuenow", "20");
    await expect(frame).not.toHaveCSS("pointer-events", "none");
    await panel.frameLocator("iframe").getByRole("button", { name: "Artifact action" }).click();
    await page.setViewportSize({ width: 720, height: 600 });
    await divider.focus();
    await page.keyboard.press("End");
    await expect(async () => {
      expect((await panel.boundingBox())!.width).toBeCloseTo(720 * 0.7, 0);
      expect((await page.locator(".app-workspace").boundingBox())!.width).toBeCloseTo(720 * 0.3, 0);
    }).toPass({ timeout: 5000 });
    await expect(page.locator(".app-workspace")).toBeVisible();
    await assertAligned();
  });
}

test.describe("touch shell alignment", () => {
  test.use({ hasTouch: true });
  test("headers contain controls at every reading size", async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 960 });
    await installDatasetReportsMock(page, {});
    await page.goto("/");
    await expect(page.getByRole("main", { name: "Chat workspace" })).toBeVisible();
    await page.evaluate(async () => {
      const path = "/src/features/canvas/canvasStore.ts";
      const store = await import(/* @vite-ignore */ path);
      store.openCanvasArtifact({ id: "touch", type: "table", title: "Sales", columns: ["Product"], rows: [["A"]] });
    });
    for (const size of ["small", "medium", "large"]) {
      await page.evaluate((value) => {
        document.documentElement.dataset.fontSize = value;
      }, size);
      const header = (await page.locator(".app-titlebar").boundingBox())!;
      const canvas = (await page.locator(".canvas-panel__header").boundingBox())!;
      expect(header.y).toEqual(canvas.y);
      expect(header.height).toEqual(canvas.height);
      for (const selector of [".app-titlebar", ".canvas-panel__header"]) {
        const region = page.locator(selector);
        const box = (await region.boundingBox())!;
        for (const button of await region.getByRole("button").all()) {
          const control = await button.boundingBox();
          if (!control) continue;
          expect(control.y).toBeGreaterThanOrEqual(box.y);
          expect(control.y + control.height).toBeLessThanOrEqual(box.y + box.height);
        }
      }
    }
  });
});

test("reading labels preview their own sizes without selecting them", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await installDatasetReportsMock(page, {});
  await page.goto("/");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const previews = page.locator(".reading-size-preview");
  await expect(previews).toHaveCount(3);
  for (const selected of ["Small", "Medium", "Large"]) {
    await page.getByRole("radio", { name: selected, exact: true }).click();
    const sizes = await previews.evaluateAll((nodes) =>
      nodes.map((node) => parseFloat(getComputedStyle(node).fontSize)),
    );
    for (const [index, size] of [12, 14, 16].entries()) expect(sizes[index]).toBeCloseTo(size, 2);
  }
  await page.getByRole("button", { name: "Add server", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Add MCP server", exact: true });
  await expect(dialog).toBeVisible();
  await expect(dialog).toHaveCSS("outline-width", "1px");
  await expect(dialog).toHaveCSS("outline-offset", "2px");
  await expect(dialog.getByText("Connect this server during chat")).toHaveCount(0);
  const frame = (await dialog.boundingBox())!;
  expect(frame.height).toBeLessThanOrEqual(page.viewportSize()!.height * 0.85);
  await page.setViewportSize({ width: 720, height: 480 });
  await expect(dialog.getByRole("button", { name: "Add server", exact: true })).toBeInViewport();
  await expect(dialog.getByRole("button", { name: "Cancel", exact: true })).toBeInViewport();
  const body = dialog.locator(".overflow-y-auto");
  expect(await body.evaluate((node) => node.scrollHeight > node.clientHeight)).toBe(true);
});
