import { expect, test } from "@playwright/test";
import { installVisualNativeMock } from "./native-mock";

for (const variant of [
  { name: "light", theme: "light" as const, viewport: { width: 1440, height: 960 } },
  { name: "narrow", theme: "dark" as const, viewport: { width: 720, height: 900 } },
]) {
  for (const discovery of ["available", "unsupported"] as const) {
    test(`private models ${variant.name} supports ${discovery} discovery`, async ({ page }) => {
      await page.setViewportSize(variant.viewport);
      await page.emulateMedia({ colorScheme: variant.theme, reducedMotion: "reduce" });
      await installVisualNativeMock(page, {
        theme: variant.theme,
        modelSettingsReady: true,
        privateModelDiscovery: discovery,
      });
      await page.goto("/");
      if (variant.name === "narrow") await page.getByRole("button", { name: "Show sidebar" }).click();
      await page.getByRole("button", { name: "Settings", exact: true }).click();
      await page.getByRole("button", { name: "Add private model" }).click();
      const dialog = page.getByRole("dialog", { name: "Add private model" });
      await expect(dialog.getByLabel(/^Context length/)).toHaveCount(0);
      if (discovery === "available") {
        await expect(dialog).toHaveScreenshot(["settings", `private-model-connect-${variant.name}.png`]);
      }
      await dialog.getByLabel(/^API key/).fill("fixture-private-key");
      await dialog.getByRole("button", { name: "Next", exact: true }).click();
      await expect(dialog.getByLabel(/^Context length/)).toBeVisible();
      if (discovery === "unsupported") {
        await expect(dialog.getByLabel(/^Model name/)).toBeFocused();
        await expect(dialog.getByLabel(/^Context length/)).toHaveValue("32768");
        await expect(dialog.getByLabel(/^Maximum output tokens/)).toHaveValue("4096");
        await expect(dialog.getByLabel(/^Temperature/)).toHaveValue("0.7");
        await expect(dialog.getByLabel(/^Top P/)).toHaveValue("0.9");
        await expect(dialog.getByText(/Suggested by xFalcon/)).toHaveCount(4);
        await dialog.getByLabel(/^Model name/).fill("ollama/private-model:v2");
        await dialog.getByLabel(/^Context length/).fill("32768");
        await dialog.getByLabel(/^Maximum output tokens/).fill("4096");
        await dialog.getByRole("switch", { name: "Supports tool calling" }).click();
      } else {
        await expect(dialog.getByLabel(/^Context length/)).toHaveValue("131072");
        await expect(dialog.getByLabel(/^Temperature/)).toHaveValue("0.6");
        await expect(dialog.getByText(/Provided by your model endpoint/)).toHaveCount(4);
        await dialog.getByRole("button", { name: /^Model(?:,|\s)/ }).click();
        await expect(page.getByRole("menuitemradio", { name: /qwen-3.5/ })).toBeVisible();
        await page.keyboard.press("Escape");
        await expect(dialog).toBeVisible();
      }
      await dialog.getByLabel(/^Display name/).fill(`Private model ${discovery}`);
      expect(await dialog.evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(0);
      await expect(dialog).toHaveScreenshot(["settings", `private-model-${discovery}-${variant.name}.png`]);
      await dialog.getByRole("button", { name: "Test and add model", exact: true }).click();
      await expect(dialog).toHaveCount(0);
      await expect(
        page.getByRole("group", { name: "Private models" }).getByText(`Private model ${discovery}`, { exact: true }),
      ).toBeVisible();
    });
  }
}
