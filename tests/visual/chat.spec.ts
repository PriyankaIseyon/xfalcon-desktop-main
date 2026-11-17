import { expect, test } from "@playwright/test";
import { installChatMocks } from "../e2e/chat-mocks";

test("chat empty state and recents region render under the visual contract", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
  await installChatMocks(page, { appearanceTheme: "light" });
  await page.goto("/");
  await expect(page.getByRole("region", { name: "Chat" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "What can I help you with, E2E Test?" })).toBeVisible();
  await expect(page.getByText("No conversations yet. Start a new chat to begin.")).toBeVisible();
  await expect(page.getByRole("textbox", { name: "Message xFalcon" })).toBeVisible();
  await expect(page.getByText(/mock rejected command/)).toHaveCount(0);
  await expect(page).toHaveScreenshot(["chat", "empty-light.png"]);
});

test("dark theme chat empty state preserves the visual contract", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.emulateMedia({ colorScheme: "dark", reducedMotion: "reduce" });
  await installChatMocks(page, { appearanceTheme: "dark" });
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await expect(page.getByRole("region", { name: "Chat" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "What can I help you with, E2E Test?" })).toBeVisible();
  await expect(page.getByText("No conversations yet. Start a new chat to begin.")).toBeVisible();

  const canvas = await page.evaluate(() =>
    getComputedStyle(document.documentElement).getPropertyValue("--xf-canvas").trim(),
  );
  expect(canvas).toBe("#111317");
  await expect(page).toHaveScreenshot(["chat", "empty-dark.png"]);
});

test("chat welcome keeps native-boundary identity and starter actions reachable at 200% text", async ({ page }) => {
  const displayName = "InternationalEnterpriseRevenueAndRetentionAnalysisWorkspace";
  await page.setViewportSize({ width: 900, height: 800 });
  await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
  await installChatMocks(page, { appearanceTheme: "light", displayName });
  await page.goto("/");

  const heading = page.getByRole("heading", { name: `What can I help you with, ${displayName}?` });
  await expect(heading).toBeVisible();
  await page.evaluate(async () => {
    await document.fonts.ready;
    document.documentElement.style.fontSize = "32px";
    void document.body.offsetHeight;
  });

  const layout = await page.evaluate(() => {
    const title = document.querySelector<HTMLElement>(".empty-conversation h1");
    const container = document.querySelector<HTMLElement>(".empty-conversation");
    if (!title || !container) throw new Error("welcome fixture did not render");
    return {
      documentOverflow: document.documentElement.scrollWidth - document.documentElement.clientWidth,
      titleOverflow: title.scrollWidth - title.clientWidth,
      overflowY: getComputedStyle(container).overflowY,
    };
  });
  expect(layout.documentOverflow).toBeLessThanOrEqual(0);
  expect(layout.titleOverflow).toBeLessThanOrEqual(0);
  expect(layout.overflowY).toBe("auto");

  const lastStarter = page.getByRole("button", { name: "Suggest three questions worth asking my data" });
  await lastStarter.scrollIntoViewIfNeeded();
  await expect(lastStarter).toBeVisible();
});

test("composer suggestions keep their UI contract at 200% text", async ({ page }) => {
  await page.setViewportSize({ width: 720, height: 900 });
  await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
  await installChatMocks(page, {
    appearanceTheme: "light",
    mcpServers: [
      {
        version: 1,
        id: "mcp-analytics",
        name: "enterprise-analytics",
        transport: "http",
        enabled: true,
      },
    ],
  });
  await page.goto("/");

  const composer = page.getByRole("textbox", { name: "Message xFalcon" });
  await page.evaluate(async () => {
    await document.fonts.ready;
    document.documentElement.style.fontSize = "32px";
  });

  await composer.fill("@");
  const palette = page.getByRole("listbox", { name: "Mention suggestions" });
  await expect(palette).toBeVisible();
  await expect(page.getByText("@mcp:enterprise-analytics")).toBeVisible();

  const paletteGeometry = await page.evaluate(() => {
    const header = document.querySelector<HTMLElement>(".composer-palette__header");
    const shortcuts = document.querySelector<HTMLElement>(".composer-palette__shortcuts");
    if (!header || !shortcuts) throw new Error("composer suggestion header did not render");
    return {
      headerOverflow: header.scrollWidth - header.clientWidth,
      shortcutsOverflow: shortcuts.scrollWidth - shortcuts.clientWidth,
    };
  });
  expect(paletteGeometry.headerOverflow).toBeLessThanOrEqual(0);
  expect(paletteGeometry.shortcutsOverflow).toBeLessThanOrEqual(0);

  await composer.fill("@does-not-exist");
  await expect(palette).toHaveText(/No matching files or resources found/);
  await composer.press("Escape");
  await expect(palette).toHaveCount(0);

  await composer.fill("@mcp:enterprise");
  await page.getByText("@mcp:enterprise-analytics").click();
  await composer.type("review it");
  await expect(composer).toHaveValue("@mcp:enterprise-analytics review it");
});

test("recovery notice surfaces when bootstrap reports an interrupted run", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
  await installChatMocks(page, {
    appearanceTheme: "light",
    recoveryNotice: "A previous response was interrupted. Reconcile or retry when ready.",
  });
  await page.goto("/");
  await expect(page.getByRole("status").filter({ hasText: "interrupted" })).toBeVisible();
  await expect(page).toHaveScreenshot(["chat", "recovery-light.png"]);
});

for (const theme of ["light", "dark"] as const) {
  test(`composer has one border and separated placeholder in ${theme}`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
    await installChatMocks(page, { appearanceTheme: theme });
    await page.goto("/");
    const input = page.getByRole("textbox", { name: "Message xFalcon" });
    const shell = page.getByRole("form", { name: "Message composer" });
    await expect(input).toBeVisible();
    await input.focus();
    await expect(shell).toHaveCSS("border-top-width", "2px");
    await expect(shell).toHaveCSS("border-top-color", theme === "light" ? "rgb(24, 114, 136)" : "rgb(94, 216, 236)");
    await expect(shell).toHaveCSS("box-shadow", "none");
    await expect(input).toHaveCSS("border-top-width", "0px");
    expect(await shell.evaluate((el) => getComputedStyle(el, "::before").content)).toBe("none");
    await expect(input).toHaveAttribute("placeholder", "\u00a0\u00a0Ask about your data");
    await expect(input).toHaveValue("");
    const focusedBox = await shell.boundingBox();
    await input.pressSequentially("Revenue");
    await expect(input).toHaveValue("Revenue");
    await input.fill("");
    await input.evaluate((el) => el.blur());
    const neutral = theme === "light" ? "rgba(15, 23, 42, 0.1)" : "rgba(255, 255, 255, 0.12)";
    await expect(shell).toHaveCSS("border-top-color", neutral);
    await expect(shell).toHaveCSS("box-shadow", "none");
    expect(await shell.boundingBox()).toEqual(focusedBox);
  });
}
