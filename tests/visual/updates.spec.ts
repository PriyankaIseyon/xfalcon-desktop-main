import { expect, test } from "@playwright/test";
import { installVisualNativeMock } from "./native-mock";

for (const theme of ["light", "dark"] as const) {
  test(`update notice stays usable across workspaces in ${theme}`, async ({ page }) => {
    await page.setViewportSize({ width: theme === "light" ? 1440 : 900, height: 900 });
    await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
    await installVisualNativeMock(page, { theme, automaticUpdate: true });
    await page.goto("/");
    await expect(page.getByRole("main", { name: "Chat workspace" })).toBeVisible();
    const notice = page.getByRole("region", { name: "Software update" });
    await expect(notice.getByRole("button", { name: "Restart to update" })).toBeVisible();
    if (theme === "dark") await page.getByRole("button", { name: "Show sidebar" }).click();
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await expect(notice).toBeVisible();
    if (theme === "light") {
      const settings = page.getByRole("button", { name: "Settings", exact: true });
      await expect(settings).toHaveAccessibleDescription("1 update available");
      const badge = page
        .getByRole("button", { name: "New update available. Open software updates" })
        .locator("span[aria-hidden]");
      await expect(badge).toHaveText("1");
      const bounds = (await badge.boundingBox())!;
      expect(bounds.width).toBe(bounds.height);
    }
    await expect(notice.locator(":scope > div").first()).toHaveCSS("justify-content", "center");
    await expect(notice.getByRole("status")).toHaveCSS("text-align", "center");
    await expect(page.getByRole("main", { name: "Settings" })).toBeVisible();
    await notice.getByRole("button", { name: "Later" }).click();
    await expect(notice).toHaveCount(0);
    if (theme === "light")
      await expect(page.getByRole("button", { name: "Settings", exact: true })).toHaveAccessibleDescription(
        "1 update available",
      );
    await page.reload();
    await expect(notice.getByRole("button", { name: "Restart to update" })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)).toBeLessThanOrEqual(0);
    await page.screenshot({ path: `/tmp/xf-update-${theme}.png` });
  });
}

for (const width of [1440, 900]) {
  test(`update badge opens software updates with pointer and keyboard at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ reducedMotion: "reduce" });
    await installVisualNativeMock(page, { automaticUpdate: true });
    await page.goto("/");
    const notice = page.getByRole("region", { name: "Software update" });
    await notice.getByRole("button", { name: "Later" }).click();
    const showSidebar = page.getByRole("button", { name: "Show sidebar" });
    if (width === 900) await showSidebar.click();

    const badge = page.getByRole("button", { name: "New update available. Open software updates" });
    await badge.hover();
    await expect(page.getByRole("tooltip")).toHaveText("New update available. Open software updates");
    await badge.click();
    await expect(page.getByRole("main", { name: "Settings" })).toBeVisible();
    const updates = page.getByRole("group", { name: "Software updates", exact: true });
    await expect(updates).toHaveAttribute("data-search-reveal", "");
    await expect(updates.getByRole("button", { name: "Restart app" })).toBeInViewport();

    // Repeating the action while Settings is open must reveal the section again.
    // SettingsView reconciles async section layout at 0/50/160/400 ms. Start
    // the second interaction after that known reveal sequence has finished.
    await page.waitForTimeout(450);
    await page.locator("[data-settings-page]").evaluate((element) => {
      element.scrollTop = 0;
    });
    await expect(updates.getByRole("button", { name: "Restart app" })).not.toBeInViewport();
    if (width === 900) await showSidebar.click();
    const settings = page.getByRole("button", { name: "Settings", exact: true });
    // Wait for the reopened drawer to settle and move the pointer away from
    // the badge before exercising keyboard focus. Tooltips close on scroll.
    await settings.hover();
    await settings.focus();
    await page.keyboard.press("Tab");
    await expect(badge).toBeFocused();
    await expect(page.getByRole("tooltip")).toHaveText("New update available. Open software updates");
    await page.keyboard.press("Enter");
    await expect(updates.getByRole("button", { name: "Restart app" })).toBeInViewport();
    expect(await page.evaluate(() => Reflect.get(window, "__visualUpdateCalls"))).toEqual([
      "update_check",
      "update_install",
    ]);
  });
}

test("Settings opens normally without an update", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await installVisualNativeMock(page);
  await page.goto("/");
  await expect(page.getByRole("button", { name: "New update available. Open software updates" })).toHaveCount(0);
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Appearance", exact: true })).toBeInViewport();
  await expect(page.locator("#settings-updates")).not.toHaveAttribute("data-search-reveal", "");
});

test("startup downloads automatically and waits for explicit restart", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await installVisualNativeMock(page, { automaticUpdate: true });
  await page.goto("/");
  const notice = page.getByRole("region", { name: "Software update" });
  await expect(notice.getByRole("button", { name: "Restart to update" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Settings", exact: true })).toHaveAccessibleDescription(
    "1 update available",
  );
  const calls = await page.evaluate(() => Reflect.get(window, "__visualUpdateCalls"));
  expect(calls).toEqual(["update_check", "update_install"]);
});

const RELEASE_NOTES = [
  "# Release highlights",
  "",
  "This release adds **verified report exports**.",
  "",
  "- Faster summaries",
  "- Clearer project links",
  "",
  "Use `release_version` to check the installed version.",
  "",
  "```sql",
  `SELECT ${"x".repeat(280)} FROM sales;`,
  "```",
  "",
  "[Read the update guide](https://example.com/releases/0.5.5)",
  "",
  "| Region with a long name | Store group with a long name | Measure with a long name | Notes with a long name |",
  "| --- | --- | --- | --- |",
  `| ${"north-region-".repeat(4)} | ${"enterprise-group-".repeat(4)} | ${"net-sales-measure-".repeat(4)} | ${"updated-report-details-".repeat(4)} |`,
].join("\n");

for (const variant of [
  { name: "light-desktop", theme: "light" as const, width: 1440 },
  { name: "dark-narrow", theme: "dark" as const, width: 600 },
]) {
  test(`release notes stay readable and contained in ${variant.name} Settings`, async ({ page }) => {
    await page.setViewportSize({ width: variant.width, height: 900 });
    await page.emulateMedia({ colorScheme: variant.theme, reducedMotion: "reduce" });
    await installVisualNativeMock(page, {
      theme: variant.theme,
      automaticUpdate: true,
      releaseNotes: RELEASE_NOTES,
    });
    await page.goto("/");

    if (variant.width <= 900) await page.getByRole("button", { name: "Show sidebar" }).click();
    const updateBadge = page.getByRole("button", { name: "New update available. Open software updates" });
    await expect(updateBadge).toBeVisible();
    await updateBadge.click();

    const settings = page.getByRole("main", { name: "Settings" });
    const updates = page.getByRole("group", { name: "Software updates", exact: true });
    await expect(settings).toBeVisible();
    await expect(updates).toHaveAttribute("data-search-reveal", "");
    await expect(updates.getByRole("heading", { name: "Release highlights" })).toBeVisible();
    await expect(updates.getByText("verified report exports", { exact: true })).toBeVisible();
    await expect(updates.getByRole("listitem")).toHaveCount(2);
    await expect(updates.getByText("release_version", { exact: true })).toBeVisible();

    const code = updates.locator(".assistant-markdown pre").first();
    await expect(code).toContainText("SELECT");
    expect(await code.evaluate((element) => element.scrollWidth - element.clientWidth)).toBeGreaterThan(0);
    const tableRegion = updates.getByRole("region", { name: "Scrollable release notes table" });
    await expect(tableRegion.getByRole("table")).toBeVisible();
    expect(await tableRegion.evaluate((element) => element.scrollWidth - element.clientWidth)).toBeGreaterThan(0);
    const link = updates.getByRole("link", { name: /Read the update guide/ });
    await expect(link).toHaveAttribute("href", "https://example.com/releases/0.5.5");
    await expect(link).toHaveAttribute("target", "_blank");

    await expect(updates.getByText("Markdown", { exact: true })).toHaveCount(0);
    await expect(updates.getByRole("button", { name: "Show source of Markdown" })).toHaveCount(0);
    await expect(updates.getByRole("button", { name: "Copy Markdown to clipboard" })).toHaveCount(0);

    await expect(updates.getByRole("button", { name: "Restart app" })).toBeVisible();
    expect(await settings.evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(0);
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth),
    ).toBeLessThanOrEqual(0);
    expect(await page.evaluate(() => Reflect.get(window, "__visualPreviewRegistrationCalls"))).toEqual([]);
    await page.screenshot({
      animations: "disabled",
      caret: "hide",
      path: `/tmp/xf-update-release-notes-${variant.name}.png`,
      scale: "css",
    });
  });
}
