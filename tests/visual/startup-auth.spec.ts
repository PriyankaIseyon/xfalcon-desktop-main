import { expect, test, type Page } from "@playwright/test";
import { installVisualNativeMock } from "./native-mock";

const STARTUP_GALLERY = "/startup-visual.html";

type StartupTheme = "light" | "dark";

async function openStartupScreen(
  page: Page,
  theme: StartupTheme,
  mock: Parameters<typeof installVisualNativeMock>[1],
): Promise<void> {
  await page.setViewportSize({ width: 1440, height: 960 });
  await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
  await installVisualNativeMock(page, { theme, ...mock });
  await page.goto("/");
  await page.evaluate(async () => document.fonts.ready);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
}

for (const theme of ["light", "dark"] as const) {
  test(`startup loading preserves the brand frame in ${theme}`, async ({ page }) => {
    await openStartupScreen(page, theme, { bootstrapPending: true });
    await expect(page.getByRole("heading", { name: "xFalcon Desktop" })).toBeVisible();
    await expect(page.getByRole("status")).toContainText("Starting xFalcon");
    await expect(page.locator(".app-mark")).toBeVisible();
    await expect(page).toHaveScreenshot(["startup-auth", `startup-loading-${theme}.png`]);
  });

  test(`startup failure preserves the brand frame in ${theme}`, async ({ page }) => {
    await openStartupScreen(page, theme, { bootstrapReject: true });
    await expect(page.getByRole("heading", { name: "Unable to start xFalcon" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Try again" })).toBeVisible();
    await expect(page.locator(".app-mark")).toBeVisible();
    await expect(page).toHaveScreenshot(["startup-auth", `startup-error-${theme}.png`]);
  });

  test(`signed-out authentication preserves the login hierarchy in ${theme}`, async ({ page }) => {
    await openStartupScreen(page, theme, { sessionStatus: "unauthenticated" });
    await expect(page.getByRole("heading", { name: "Sign in to xFalcon" })).toBeVisible();
    await expect(page.getByRole("button", { name: /Continue in browser/ })).toBeVisible();

    const headingSize = await page
      .getByRole("heading", { name: "Sign in to xFalcon" })
      .evaluate((element) => Number.parseFloat(getComputedStyle(element).fontSize));
    const titleRamp = await page.evaluate(() => {
      const probe = document.createElement("span");
      probe.style.position = "absolute";
      probe.style.visibility = "hidden";
      probe.style.fontSize = "var(--xf-type-title)";
      document.body.append(probe);
      const size = Number.parseFloat(getComputedStyle(probe).fontSize);
      probe.remove();
      return size;
    });
    // HeroTitle uses the display role; the old `.app-startup h1` override forced
    // it down to the plain title step. The rendered px has to stay above title.
    expect(headingSize).toBeGreaterThan(titleRamp + 2);

    await expect(page).toHaveScreenshot(["startup-auth", `login-${theme}.png`]);
  });

  test(`authenticating state preserves the startup-family frame in ${theme}`, async ({ page }) => {
    await openStartupScreen(page, theme, { sessionStatus: "authenticating" });
    await expect(page.getByRole("heading", { name: "Continuing sign-in…" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Cancel sign-in" })).toBeVisible();
    await expect(page.locator(".app-mark")).toBeVisible();
    await expect(page).toHaveScreenshot(["startup-auth", `auth-authenticating-${theme}.png`]);
  });

  test(`app recovery preserves the startup-family frame in ${theme}`, async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 960 });
    await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
    await installVisualNativeMock(page, { theme });
    await page.goto(STARTUP_GALLERY);
    await page.evaluate(async () => document.fonts.ready);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await expect(page.getByRole("heading", { name: "xFalcon needs to recover" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Reload workspace" })).toBeVisible();
    await expect(page.locator(".app-mark")).toBeVisible();
    await expect(page).toHaveScreenshot(["startup-auth", `recovery-${theme}.png`]);
  });
}
