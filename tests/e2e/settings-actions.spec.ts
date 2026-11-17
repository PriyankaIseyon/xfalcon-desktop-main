import { expect, test } from "@playwright/test";
import { installVisualNativeMock } from "../visual/native-mock";

for (const variant of [
  { theme: "light" as const, width: 1440, readingSize: "Small" },
  { theme: "dark" as const, width: 720, readingSize: "Large" },
]) {
  test(`Settings removal actions match in ${variant.theme} at ${variant.width}px`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width: variant.width, height: 960 });
    await page.emulateMedia({ colorScheme: variant.theme, reducedMotion: "reduce" });
    await installVisualNativeMock(page, {
      theme: variant.theme,
      modelSettingsReady: true,
      mcpRecovery: true,
      providerCredentials: [
        {
          version: 1,
          id: "cred-settings",
          provider: "openrouter",
          display_name: "openrouter_1",
          status: "ready",
          is_default: true,
        },
      ],
      skills: [
        {
          version: 1,
          name: "ida-dashboard",
          description: "Build interactive dashboards.",
          declared_version: "1.0.0",
          enabled: true,
          content_hash: "a".repeat(64),
          bundled_files: 0,
        },
      ],
    });
    await page.goto("/");
    if (variant.width === 720) await page.getByRole("button", { name: "Show sidebar" }).click();
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    const settings = page.getByRole("main", { name: "Settings" });
    await settings
      .getByRole("radiogroup", { name: "Reading size" })
      .getByRole("radio", { name: variant.readingSize })
      .click();
    const removals = settings.getByRole("button", { name: /^Remove(?: |$)/ });
    await expect(removals).toHaveCount(5);
    await expect(settings.getByText("ida-dashboard", { exact: true })).toBeVisible();

    const readStyle = (element: HTMLElement) => {
      const style = getComputedStyle(element);
      const icon = element.querySelector("svg");
      return {
        label: element.textContent?.trim(),
        color: style.color,
        background: style.backgroundColor,
        borderColor: style.borderColor,
        borderWidth: style.borderWidth,
        radius: style.borderRadius,
        height: style.height,
        width: style.width,
        fontSize: style.fontSize,
        fontWeight: style.fontWeight,
        padding: style.padding,
        cursor: style.cursor,
        outline: style.outline,
        icon: icon?.getAttribute("class"),
        iconWidth: icon ? getComputedStyle(icon).width : null,
      };
    };
    const restingStyles = await Promise.all((await removals.all()).map((button) => button.evaluate(readStyle)));
    expect(new Set(restingStyles.map((style) => JSON.stringify(style))).size).toBe(1);
    expect(restingStyles[0].label).toBe("Remove");
    expect(restingStyles[0].borderWidth).toBe("1px");
    expect(Number.parseFloat(restingStyles[0].width)).toBeGreaterThan(60);

    const hoverStyles = [];
    const focusStyles = [];
    const confirmations = [
      { title: "Remove “QWEN”?", label: "Remove model" },
      { title: "Remove “openrouter_1”?", label: "Remove key" },
      { title: "Remove “Workspace MCP server”?", label: "Remove server" },
      { title: "Remove “Analytics”?", label: "Remove server" },
      { title: "Remove “ida-dashboard”?", label: "Remove skill" },
    ];
    for (let index = 0; index < confirmations.length; index++) {
      const button = removals.nth(index);
      await button.hover();
      await expect
        .poll(() => button.evaluate((element) => getComputedStyle(element).backgroundColor))
        .not.toBe(restingStyles[index].background);
      await button.evaluate((element) => Promise.all(element.getAnimations().map((animation) => animation.finished)));
      hoverStyles.push(await button.evaluate(readStyle));
      await page.mouse.move(0, 0);
      await expect(button).toHaveCSS("background-color", restingStyles[index].background);
      await page.keyboard.press("Tab");
      await button.focus();
      focusStyles.push(await button.evaluate(readStyle));
      await page.keyboard.press("Enter");
      const dialog = page.getByRole("alertdialog", { name: confirmations[index].title });
      await expect(dialog).toBeVisible();
      await expect(dialog.getByRole("button", { name: confirmations[index].label, exact: true })).toBeEnabled();
      const cancel = dialog.getByRole("button", { name: "Cancel", exact: true });
      await expect(cancel).toBeFocused();
      await cancel.click();
      await expect(dialog).toHaveCount(0);
      await expect(button).toBeFocused();
      await button.blur();
    }
    expect(new Set(hoverStyles.map((style) => JSON.stringify(style))).size).toBe(1);
    expect(new Set(focusStyles.map((style) => JSON.stringify(style))).size).toBe(1);
    expect(focusStyles[0].outline).toContain("2px");
    expect(hoverStyles[0].background).not.toBe(restingStyles[0].background);
    await expect(removals).toHaveCount(5);
    expect(await settings.evaluate((element) => element.scrollWidth - element.clientWidth)).toBeLessThanOrEqual(0);
    await settings.locator("[data-settings-page]").evaluate((element) => {
      element.scrollTop = 0;
    });
    await page.screenshot({ path: testInfo.outputPath("settings-actions.png") });
  });
}
