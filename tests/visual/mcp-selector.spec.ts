import { expect, test } from "@playwright/test";
import { installVisualNativeMock } from "./native-mock";

for (const theme of ["light", "dark"] as const) {
  test(`MCP selection keeps a draft only when the server stays the same (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 1100, height: 800 });
    await installVisualNativeMock(page, { theme, modelSettingsReady: true, updatesUpToDate: true });
    await page.goto("/");

    const composer = page.getByRole("form", { name: "Message composer" });
    const message = composer.getByRole("textbox");
    const selector = composer.getByRole("button", { name: "MCP server", exact: true });
    await expect(selector).toHaveText("All servers");
    await message.fill("This draft belongs to All servers.");

    await selector.click();
    await page.getByRole("menuitemradio", { name: "Tenant Data Analyst", exact: true }).click();
    await expect(selector).toHaveText("Tenant Data Analyst");
    await expect(message).toHaveValue("");

    await message.fill("Keep this draft for the selected server.");
    await selector.focus();
    await page.keyboard.press("Space");
    const sameServer = page.getByRole("menuitemradio", { name: "Tenant Data Analyst", exact: true });
    await expect(sameServer).toHaveAttribute("aria-checked", "true");
    await sameServer.focus();
    await page.keyboard.press("Enter");
    await expect(message).toHaveValue("Keep this draft for the selected server.");

    await selector.click();
    await page.getByRole("menuitemradio", { name: "All servers", exact: true }).click();
    await expect(selector).toHaveText("All servers");
    await expect(message).toHaveValue("");
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  });
}
