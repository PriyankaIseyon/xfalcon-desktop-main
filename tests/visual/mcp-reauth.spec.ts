import { expect, test } from "@playwright/test";
import { installVisualNativeMock } from "./native-mock";

for (const theme of ["light", "dark"] as const) {
  test(`MCP recovery in ${theme} prompts globally and refreshes open Settings`, async ({ page }) => {
    await page.setViewportSize({ width: 960, height: 720 });
    await page.emulateMedia({ reducedMotion: "reduce", colorScheme: theme });
    await installVisualNativeMock(page, { theme, mcpRecovery: true });
    await page.goto("/");
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await expect(
      page
        .getByRole("group", { name: "MCP servers", exact: true })
        .getByRole("button", { name: "Sign in with this server", exact: true }),
    ).toBeVisible();
    await expect
      .poll(() => page.evaluate(() => (window as unknown as { __raiseMcpReauth: () => number }).__raiseMcpReauth()))
      .toBe(1);
    const dialog = page.getByRole("alertdialog", { name: "Reconnect Analytics" });
    await expect(dialog).toBeVisible();
    await expect(dialog).toHaveScreenshot(["mcp", `reauth-${theme}.png`]);
    await dialog.getByRole("button", { name: "Reconnect", exact: true }).click();
    await expect(dialog).not.toBeVisible();
    await expect(
      page
        .getByRole("group", { name: "MCP servers", exact: true })
        .getByRole("button", { name: "Sign out", exact: true }),
    ).toBeVisible();
  });
}
