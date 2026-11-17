import { expect, test, type Locator, type Page } from "@playwright/test";
import { installVisualNativeMock } from "./native-mock";

async function openSettings(page: Page, width = 1440, theme: "light" | "dark" = "light") {
  await page.setViewportSize({ width, height: 960 });
  await installVisualNativeMock(page, { theme, modelSettingsReady: true, updatesUpToDate: true });
  await page.goto("/");
  await expect(page.getByRole("main", { name: "Chat workspace" })).toBeVisible();
  if (width <= 900) await page.getByRole("button", { name: "Show sidebar" }).click();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(page.getByRole("main", { name: "Settings" })).toBeVisible();
  await page.evaluate(() => document.fonts.ready.then(() => undefined));
}

async function expectThumbAligned(group: Locator) {
  await expect
    .poll(async () =>
      group.evaluate((element) => {
        const selected = element.querySelector('[aria-checked="true"], [aria-selected="true"]');
        const thumb = element.querySelector("[data-elastic-thumb]");
        if (!selected || !thumb) return Number.POSITIVE_INFINITY;
        const target = selected.getBoundingClientRect();
        const actual = thumb.getBoundingClientRect();
        return Math.max(
          Math.abs(target.left - actual.left),
          Math.abs(target.width - actual.width),
          Math.abs(target.top - actual.top),
        );
      }),
    )
    .toBeLessThan(1);
}

async function recordSelection(group: Locator) {
  await group.evaluate((element) => {
    const samples: { overflow: number; center: number }[] = [];
    Reflect.set(window, "__selectionFrames", { samples, done: false });
    const until = performance.now() + 650;
    const sample = () => {
      const rect = element.getBoundingClientRect();
      const thumb = element.querySelector("[data-elastic-thumb]")!.getBoundingClientRect();
      samples.push({
        overflow: element.scrollWidth - element.clientWidth,
        center: (thumb.left + thumb.width / 2 - rect.left) / rect.width,
      });
      if (performance.now() < until) requestAnimationFrame(sample);
      else Reflect.set(window, "__selectionFrames", { samples, done: true });
    };
    requestAnimationFrame(sample);
  });
}

async function expectSmoothSelection(page: Page) {
  await expect.poll(() => page.evaluate(() => Reflect.get(window, "__selectionFrames").done)).toBe(true);
  const frames: { overflow: number; center: number }[] = await page.evaluate(
    () => Reflect.get(window, "__selectionFrames").samples,
  );
  expect(Math.max(...frames.map((frame) => frame.overflow))).toBe(0);
  const start = frames[0]!.center;
  const end = frames.at(-1)!.center;
  const minimum = Math.min(start, end) + 0.02;
  const maximum = Math.max(start, end) - 0.02;
  expect(frames.filter((frame) => frame.center > minimum && frame.center < maximum).length).toBeGreaterThan(5);
  const steps = frames.slice(1).map((frame, index) => Math.abs(frame.center - frames[index]!.center));
  expect(Math.max(...steps)).toBeLessThan(Math.abs(end - start) * 0.45);
}

async function openThanks(page: Page) {
  const version = page.getByRole("group", { name: "Software updates", exact: true }).getByText(/^v0\./);
  for (let index = 0; index < 6; index += 1) await version.click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await version.click();
  const dialog = page.getByRole("dialog", { name: "Thank you for being here." });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("Built with care by the xFalcon team.", { exact: true })).toBeVisible();
  return dialog;
}

async function observeGpu(page: Page) {
  await page.addInitScript(() => {
    Reflect.set(window, "__asciiDraws", 0);
    Reflect.set(window, "__asciiWaveSeen", false);
    Reflect.set(window, "__frameRequests", 0);
    const frame = window.requestAnimationFrame;
    window.requestAnimationFrame = (callback) => {
      Reflect.set(window, "__frameRequests", Number(Reflect.get(window, "__frameRequests")) + 1);
      return frame.call(window, callback);
    };
    if (typeof WebGL2RenderingContext === "undefined") return;
    const prototype = WebGL2RenderingContext.prototype;
    const names = new WeakMap<WebGLUniformLocation, string>();
    const locate = prototype.getUniformLocation;
    prototype.getUniformLocation = function (program, name) {
      const location = locate.call(this, program, name);
      if (location) names.set(location, name);
      return location;
    };
    const uniform = prototype.uniform1f;
    prototype.uniform1f = function (location, value) {
      if (location && names.get(location) === "uClickActive" && value > 0) {
        Reflect.set(window, "__asciiWaveSeen", true);
      }
      return uniform.call(this, location, value);
    };
    const draw = prototype.drawArrays;
    prototype.drawArrays = function (mode, first, count) {
      Reflect.set(window, "__asciiDraws", Number(Reflect.get(window, "__asciiDraws")) + 1);
      return draw.call(this, mode, first, count);
    };
  });
}

for (const variant of [
  { width: 1440, theme: "light" as const, motion: "no-preference" as const },
  { width: 560, theme: "dark" as const, motion: "reduce" as const },
]) {
  test(`Settings controls preserve selection and fit at ${variant.width}px`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: variant.theme, reducedMotion: variant.motion });
    await openSettings(page, variant.width, variant.theme);
    const theme = page.getByRole("radiogroup", { name: "Theme", exact: true });
    const reading = page.getByRole("radiogroup", { name: "Reading size", exact: true });
    await expectThumbAligned(theme);
    await theme.getByRole("radio", { name: "Dark", exact: true }).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await expectThumbAligned(theme);
    await reading.getByRole("radio", { name: "Large", exact: true }).click();
    await expectThumbAligned(reading);
    const start = (await reading.getByRole("radio", { name: "Large", exact: true }).boundingBox())!;
    const end = (await reading.getByRole("radio", { name: "Small", exact: true }).boundingBox())!;
    await page.mouse.move(start.x + start.width / 2, start.y + start.height / 2);
    await page.mouse.down();
    await page.mouse.move(end.x + end.width / 2, end.y + end.height / 2, { steps: 12 });
    await page.mouse.up();
    await expect(reading.getByRole("radio", { name: "Small", exact: true })).toHaveAttribute("aria-checked", "true");
    // A fresh click immediately after a drag must never be eaten as its synthetic click.
    await reading.getByRole("radio", { name: "Medium", exact: true }).click();
    await expect(reading.getByRole("radio", { name: "Medium", exact: true })).toHaveAttribute("aria-checked", "true");
    await expectThumbAligned(reading);
    await page.setViewportSize({ width: variant.width === 1440 ? 1000 : 600, height: 960 });
    await expectThumbAligned(reading);
    expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(0);
  });

  test(`update success is centered and temporary at ${variant.width}px`, async ({ page }, testInfo) => {
    await page.emulateMedia({ colorScheme: variant.theme, reducedMotion: "reduce" });
    await openSettings(page, variant.width, variant.theme);
    const updates = page.getByRole("group", { name: "Software updates", exact: true });
    const button = updates.getByRole("button", { name: "Check for updates", exact: true });
    const beforeButton = (await button.boundingBox())!;
    await button.click();
    const message = updates.getByRole("status");
    await expect(message).toHaveText("xFalcon is up to date.");
    const presentation = await message.evaluate((element) => {
      const row = element.closest("[data-settings-row]");
      const button = row?.querySelector("button");
      const style = getComputedStyle(element);
      return {
        background: style.backgroundColor,
        color: style.color,
        fontSize: style.fontSize,
        captionSize: getComputedStyle(row!.querySelector(".settings-row__hint")!).fontSize,
        inRow: !!row,
        beforeButton: !!button && !!(element.compareDocumentPosition(button) & Node.DOCUMENT_POSITION_FOLLOWING),
        ribbon: element.classList.contains("settings-group__state"),
      };
    });
    expect(presentation).toMatchObject({ inRow: true, beforeButton: true, ribbon: false });
    expect(presentation.background).not.toBe("rgba(0, 0, 0, 0)");
    expect(presentation.color).not.toBe(presentation.background);
    expect(Number.parseFloat(presentation.fontSize)).toBeGreaterThan(Number.parseFloat(presentation.captionSize));
    const visibleButton = (await button.boundingBox())!;
    expect(Math.abs(visibleButton.x - beforeButton.x)).toBeLessThan(1);
    if (variant.width > 1000) {
      const messageBounds = (await message.boundingBox())!;
      const buttonBounds = (await button.boundingBox())!;
      const versionBounds = (await updates.getByText(/^v0\./).boundingBox())!;
      const rowBounds = (await updates.locator("[data-settings-row]").first().boundingBox())!;
      expect(Math.abs(messageBounds.x + messageBounds.width / 2 - rowBounds.x - rowBounds.width / 2)).toBeLessThan(1);
      expect(messageBounds.x).toBeGreaterThan(versionBounds.x + versionBounds.width);
      expect(messageBounds.x + messageBounds.width).toBeLessThanOrEqual(buttonBounds.x);
    }
    expect(await updates.evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(0);
    await updates.screenshot({ path: testInfo.outputPath(`update-success-${variant.theme}.png`) });
    await expect(message).toBeHidden({ timeout: 6000 });
    const afterButton = (await button.boundingBox())!;
    expect(Math.abs(afterButton.x - beforeButton.x)).toBeLessThan(1);
    await expect(updates.getByText(/Last checked: .*Succeeded/)).toBeVisible();
  });

  test(`thank-you artwork is usable and stops on close in ${variant.theme}`, async ({ page }, testInfo) => {
    await observeGpu(page);
    await page.emulateMedia({ colorScheme: variant.theme, reducedMotion: variant.motion });
    await openSettings(page, variant.width, variant.theme);
    const version = page.getByRole("group", { name: "Software updates", exact: true }).getByText(/^v0\./);
    await version.scrollIntoViewIfNeeded();
    const versionBounds = (await version.boundingBox())!;
    const dialog = await openThanks(page);
    await expect(dialog.getByRole("button", { name: "Close", exact: true })).toBeFocused();
    for (let index = 0; index < 8; index += 1) {
      await page.mouse.click(versionBounds.x + versionBounds.width / 2, versionBounds.y + versionBounds.height / 2);
    }
    await page.mouse.click(5, 5);
    await expect(dialog).toBeVisible();
    const art = dialog.locator(".gradient-ascii");
    await expect(art).toHaveAttribute("data-ascii-ready", "true");
    await page.evaluate(() => document.fonts.ready.then(() => undefined));
    await page.screenshot({ path: testInfo.outputPath(`thanks-${variant.theme}.png`) });
    expect(await dialog.evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(0);
    if (variant.motion === "no-preference") {
      await page.waitForTimeout(5100);
      await expect(dialog).toBeVisible();
      await dialog.getByRole("heading").click();
      await expect.poll(() => page.evaluate(() => Reflect.get(window, "__asciiWaveSeen"))).toBe(true);
      await page.keyboard.press("Escape");
    } else {
      await page.waitForTimeout(150);
      const drawn = await page.evaluate(() => Reflect.get(window, "__asciiDraws"));
      await dialog.getByRole("heading").click();
      await page.waitForTimeout(150);
      expect(await page.evaluate(() => Reflect.get(window, "__asciiDraws"))).toBe(drawn);
      expect(await page.evaluate(() => Reflect.get(window, "__asciiWaveSeen"))).toBe(false);
      await dialog.getByRole("button", { name: "Close", exact: true }).click();
    }
    await expect(dialog).toHaveCount(0);
    const stopped = await page.evaluate(() => Reflect.get(window, "__asciiDraws"));
    await page.waitForTimeout(150);
    expect(await page.evaluate(() => Reflect.get(window, "__asciiDraws"))).toBe(stopped);
  });
}

test("tabs keep one panel and align after rapid selection", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.goto("/primitives.html");
  const tabs = page.getByRole("tablist", { name: "Sections", exact: true });
  await expectThumbAligned(tabs);
  for (const label of ["Two", "One", "Two"]) await tabs.getByRole("tab", { name: label, exact: true }).click();
  await expectThumbAligned(tabs);
  const active = tabs.getByRole("tab", { name: "Two", exact: true });
  const id = await active.getAttribute("aria-controls");
  expect(id).toBeTruthy();
  await expect(page.locator(`[id="${id}"]`)).toHaveAttribute("role", "tabpanel");
  await expect(tabs.getByRole("tab", { name: "One", exact: true })).not.toHaveAttribute("aria-controls");
});

test("selection moves smoothly without transient scrollbars", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await openSettings(page);
  const reading = page.getByRole("radiogroup", { name: "Reading size", exact: true });
  await reading.getByRole("radio", { name: "Small", exact: true }).click();
  await expectThumbAligned(reading);
  for (const label of ["Large", "Small"]) {
    await recordSelection(reading);
    await reading.getByRole("radio", { name: label, exact: true }).click();
    await expectSmoothSelection(page);
    await expectThumbAligned(reading);
  }
  await page.goto("/primitives.html");
  await page.evaluate(() => document.fonts.ready.then(() => undefined));
  for (const [role, name, choice] of [
    ["radiogroup", "View", "Second"],
    ["tablist", "Sections", "Two"],
  ] as const) {
    const group = page.getByRole(role, { name, exact: true });
    await group.scrollIntoViewIfNeeded();
    await expectThumbAligned(group);
    await recordSelection(group);
    await group.getByRole(role === "tablist" ? "tab" : "radio", { name: choice, exact: true }).click();
    await expectSmoothSelection(page);
    await expectThumbAligned(group);
  }
});

test("thank-you controls survive unavailable WebGL", async ({ page }) => {
  await page.addInitScript(() => {
    const getContext = HTMLCanvasElement.prototype.getContext;
    Object.defineProperty(HTMLCanvasElement.prototype, "getContext", {
      value: function (this: HTMLCanvasElement, ...args: unknown[]) {
        return args[0] === "webgl2" ? null : Reflect.apply(getContext, this, args);
      },
    });
  });
  await openSettings(page);
  const dialog = await openThanks(page);
  await expect(dialog.locator(".gradient-ascii")).toHaveAttribute("data-ascii-ready", "false");
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await expect(dialog).toHaveCount(0);
});

test("a rendering failure cannot restart the animation loop", async ({ page }) => {
  await observeGpu(page);
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await openSettings(page);
  const dialog = await openThanks(page);
  await expect(dialog.locator(".gradient-ascii")).toHaveAttribute("data-ascii-ready", "true");
  await page.evaluate(() => {
    WebGL2RenderingContext.prototype.getError = function () {
      return this.OUT_OF_MEMORY;
    };
  });
  await expect(dialog.locator(".gradient-ascii")).toHaveAttribute("data-ascii-ready", "false");
  const stopped = await page.evaluate(() => Reflect.get(window, "__frameRequests"));
  await dialog.getByRole("heading").click();
  await page.waitForTimeout(150);
  expect(await page.evaluate(() => Reflect.get(window, "__frameRequests"))).toBe(stopped);
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await expect(dialog).toHaveCount(0);
});
