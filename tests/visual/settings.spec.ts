import { expect, test } from "@playwright/test";
import { installVisualNativeMock } from "./native-mock";

for (const variant of [
  { name: "light", theme: "light" as const, viewport: { width: 1440, height: 960 } },
  { name: "narrow", theme: "dark" as const, viewport: { width: 720, height: 900 } },
]) {
  test(`settings ${variant.name} keeps its real consumer states reachable`, async ({ page }) => {
    await page.setViewportSize(variant.viewport);
    await page.emulateMedia({ colorScheme: variant.theme, reducedMotion: "reduce" });
    await installVisualNativeMock(page, { theme: variant.theme, modelSettingsReady: true });
    await page.goto("/");

    if (variant.name === "narrow") {
      await page.getByRole("button", { name: "Show sidebar" }).click();
    }
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    const settings = page.getByRole("main", { name: "Settings" });
    await expect(settings).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", variant.theme);
    await expect(settings.getByRole("heading", { name: "Settings" })).toBeVisible();
    const agentCapabilities = settings.getByRole("region", { name: "Agent capabilities", exact: true });
    const suggestions = agentCapabilities.getByRole("switch", { name: "Try next suggestions" });
    await expect(suggestions).toHaveAttribute("aria-checked", "true");
    await suggestions.focus();
    await page.keyboard.press("Space");
    await expect(suggestions).toHaveAttribute("aria-checked", "false");
    await page.keyboard.press("Space");
    await expect(suggestions).toHaveAttribute("aria-checked", "true");
    await suggestions.blur();
    const modelsConnections = settings.getByRole("region", { name: "Models & connections" });
    const defaultModel = modelsConnections.getByRole("button", { name: "Default model" });
    await expect(defaultModel).toContainText("QWEN");
    // Focusing the switch above scrolled the page down to Agent capabilities. Bring
    // the select back into view first: a scroll that lands after the click closes the menu.
    await defaultModel.scrollIntoViewIfNeeded();
    await defaultModel.click();
    await expect(page.getByRole("menuitemradio")).toHaveCount(5);
    await expect(page.getByRole("menuitemradio", { checked: true })).toHaveCount(1);
    await page.getByRole("menuitemradio", { name: /Sonnet/ }).click();
    await expect(defaultModel).toContainText("Sonnet");
    await defaultModel.click();
    await page.getByRole("menuitemradio", { name: /QWEN/ }).click();
    await expect(defaultModel).toContainText("QWEN");
    await expect(settings.getByText("No provider keys")).toBeVisible();
    await expect(settings.getByText("No additional servers")).toBeVisible();
    await expect(settings.getByText("No skills imported")).toBeVisible();

    const overflow = await settings.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);
    await expect(page).toHaveScreenshot(["settings", `${variant.name}.png`]);

    const mcp = settings.getByRole("group", { name: "MCP servers", exact: true });
    await mcp.scrollIntoViewIfNeeded();
    await expect(mcp.getByText("Workspace MCP server", { exact: true })).toBeVisible();
    await expect(mcp.getByText("Signed in", { exact: true })).toBeVisible();
    await expect(mcp.getByText("https://tenant.example/mcp")).toBeVisible();
    await expect(mcp.getByText("Not being used on this device")).toHaveCount(0);
    await expect(mcp).toHaveScreenshot(["settings", `mcp-workspace-${variant.name}.png`]);

    const application = settings.getByRole("region", { name: "Application", exact: true });
    await application.scrollIntoViewIfNeeded();
    await expect(application.getByText(/xFalcon Desktop/)).toBeVisible();
    await expect(application.getByRole("button", { name: "Check for updates" })).toBeVisible();
    await expect(application.getByText("Revision", { exact: true })).toHaveCount(0);
    await expect(application.getByRole("heading", { name: /Current build|Software updates/ })).toHaveCount(0);
    expect(await application.evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(0);
  });

  test(`settings ${variant.name} replaces the workspace MCP connection`, async ({ page }) => {
    await page.setViewportSize(variant.viewport);
    await page.emulateMedia({ colorScheme: variant.theme, reducedMotion: "reduce" });
    await installVisualNativeMock(page, { theme: variant.theme, modelSettingsReady: true });
    await page.goto("/");
    if (variant.name === "narrow") await page.getByRole("button", { name: "Show sidebar" }).click();
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    const mcp = page.getByRole("group", { name: "MCP servers", exact: true });
    await mcp.getByRole("button", { name: "Remove Workspace MCP server", exact: true }).click();
    const confirmation = page.getByRole("alertdialog");
    await expect(confirmation).toContainText("The xFalcon account stays signed in");
    await confirmation.getByRole("button", { name: "Remove server", exact: true }).click();
    await expect(mcp.getByText("Workspace MCP server", { exact: true })).toHaveCount(0);
    await expect(mcp.getByRole("button", { name: "Restore workspace MCP server" })).toBeVisible();
    await mcp.getByRole("button", { name: "Add server", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "Add MCP server" });
    // Required fields carry ", required" in their label, so each is matched by
    // prefix rather than exactly.
    await dialog.getByLabel(/^Name/).fill("custom-analytics");
    await dialog.getByLabel(/^Server address/).fill("https://custom.example/mcp");
    await dialog.getByLabel(/^Client ID/).fill("desktop-client");
    await dialog.getByLabel(/^Client secret/).fill("fixture-secret");
    expect(await dialog.evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(0);
    await expect(dialog).toHaveScreenshot(["settings", `mcp-add-oauth-${variant.name}.png`]);
    await dialog.getByRole("button", { name: "Add server", exact: true }).click();
    await expect(dialog).toHaveCount(0);
    await expect(mcp.getByText("custom-analytics", { exact: true })).toBeVisible();
    await expect(mcp.getByText("Signed in on this device", { exact: true })).toBeVisible();

    // The reported regression, end to end: reopening the sheet used to show an
    // empty Client ID over the value just saved, which reads as "it never
    // saved". The ID comes back, and the saved secret is stated and stood in for
    // rather than shown.
    await mcp.getByRole("button", { name: "Edit custom-analytics", exact: true }).click();
    const edit = page.getByRole("dialog", { name: "Edit MCP server" });
    await expect(edit.getByLabel(/^Client ID/)).toHaveValue("desktop-client");
    await expect(edit.getByLabel(/^Client secret/)).toHaveValue("");
    await expect(edit.getByLabel(/^Client secret/)).toHaveAttribute("placeholder", /\u2022+/);
    await expect(edit.getByText(/A secret is saved on this device for this client ID/)).toBeVisible();
    await edit.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(edit).toHaveCount(0);

    await mcp.getByRole("button", { name: "Restore workspace MCP server" }).click();
    await expect(mcp.getByText("Workspace MCP server", { exact: true })).toBeVisible();
    await expect(mcp.getByText("custom-analytics", { exact: true })).toBeVisible();
  });

  test(`settings ${variant.name} names OpenRouter keys automatically`, async ({ page }) => {
    await page.setViewportSize(variant.viewport);
    await page.emulateMedia({ colorScheme: variant.theme, reducedMotion: "reduce" });
    await installVisualNativeMock(page, { theme: variant.theme, modelSettingsReady: true });
    await page.goto("/");
    if (variant.name === "narrow") {
      await page.getByRole("button", { name: "Show sidebar" }).click();
    }
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await page.getByRole("button", { name: "Add provider key" }).click();
    const dialog = page.getByRole("dialog", { name: "Add provider key" });
    await expect(dialog.getByText("Key name: openai_1")).toBeVisible();
    await dialog.getByRole("radio", { name: "OpenRouter" }).click();
    await expect(dialog.getByText("Key name: openrouter_1")).toBeVisible();
    await expect(dialog.getByRole("switch", { name: "Default key for this provider" })).toBeVisible();
    await expect(dialog.getByLabel("Label", { exact: true })).toHaveCount(0);
    await dialog.getByLabel("API key", { exact: true }).fill("sk-or-visual-test-key");
    await expect(dialog.getByRole("button", { name: "Save key" })).toBeEnabled();
    expect(await dialog.evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(0);
    await expect(page).toHaveScreenshot(["settings", `provider-add-${variant.name}.png`]);
    await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
    await page.getByRole("button", { name: "Add provider key" }).click();
    await expect(dialog.getByLabel("API key", { exact: true })).toHaveValue("");
  });
}
