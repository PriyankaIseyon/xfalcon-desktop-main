import { readFileSync } from "node:fs";
import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { installVisualNativeMock } from "./native-mock";

type VisualVariant = {
  name: string;
  theme: "light" | "dark";
  viewport: { width: number; height: number };
  contrast?: "more";
  expectedCanvas: string;
  populatedSidebar?: boolean;
};

const variants: VisualVariant[] = [
  {
    name: "light",
    theme: "light",
    viewport: { width: 1440, height: 960 },
    expectedCanvas: "#ffffff",
  },
  {
    name: "dark",
    theme: "dark",
    viewport: { width: 1440, height: 960 },
    expectedCanvas: "#111317",
  },
  {
    name: "narrow",
    theme: "light",
    viewport: { width: 720, height: 900 },
    expectedCanvas: "#ffffff",
  },
  {
    name: "high-contrast",
    theme: "light",
    viewport: { width: 1440, height: 960 },
    contrast: "more",
    expectedCanvas: "#ffffff",
  },
  {
    name: "populated-sidebar",
    theme: "light",
    viewport: { width: 1440, height: 960 },
    expectedCanvas: "#ffffff",
    populatedSidebar: true,
  },
];

function normalizeHex(hex: string): string {
  const normalized = hex.toLowerCase();
  if (/^#[0-9a-f]{3}$/u.test(normalized)) {
    return `#${[...normalized.slice(1)].map((channel) => channel.repeat(2)).join("")}`;
  }
  return normalized;
}

function relativeLuminance(hex: string): number {
  const normalized = normalizeHex(hex);
  const channels = normalized
    .slice(1)
    .match(/.{2}/g)
    ?.map((channel) => Number.parseInt(channel, 16) / 255)
    .map((channel) => (channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4));
  if (!channels || channels.length !== 3) throw new Error(`Expected a three- or six-digit hex color, received ${hex}`);
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
}

function contrastRatio(foreground: string, background: string): number {
  const foregroundLuminance = relativeLuminance(foreground);
  const backgroundLuminance = relativeLuminance(background);
  return (
    (Math.max(foregroundLuminance, backgroundLuminance) + 0.05) /
    (Math.min(foregroundLuminance, backgroundLuminance) + 0.05)
  );
}

async function installNativeBoundaryMock(
  page: Page,
  variant: VisualVariant,
  { arrivalPending = false }: { arrivalPending?: boolean } = {},
): Promise<void> {
  await installVisualNativeMock(page, {
    theme: variant.theme,
    populatedSidebar: variant.populatedSidebar ?? false,
    arrivalPending,
  });
}

async function openShell(page: Page, variant: VisualVariant): Promise<void> {
  await page.setViewportSize(variant.viewport);
  await page.emulateMedia({
    colorScheme: variant.theme,
    contrast: variant.contrast ?? "no-preference",
    reducedMotion: "reduce",
  });
  await installNativeBoundaryMock(page, variant);
  await page.goto("/");
  await expect(page.getByRole("main", { name: "Chat workspace" })).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-theme", variant.theme);
  await page.evaluate(async () => document.fonts.ready);
}

async function attachRenderedShell(page: Page, testInfo: TestInfo, name: string): Promise<void> {
  const screenshotPath = testInfo.outputPath(`shell-${name}.png`);
  await page.screenshot({
    animations: "disabled",
    caret: "hide",
    fullPage: false,
    path: screenshotPath,
    scale: "css",
  });
  await testInfo.attach(`shell-${name}.png`, {
    path: screenshotPath,
    contentType: "image/png",
  });
}

for (const variant of variants) {
  test(`${variant.name} shell preserves the chat-first layout contract`, async ({ page }, testInfo) => {
    await openShell(page, variant);

    const workspace = page.getByRole("main", { name: "Chat workspace" });
    await expect(page.getByRole("heading", { name: "What can I help you with, Visual Test?" })).toBeVisible();
    await expect(page.getByRole("textbox", { name: "Message xFalcon" })).toBeVisible();
    await expect(page.getByRole("complementary", { name: "Artifact canvas" })).toHaveCount(0);
    await expect(page.getByText("MCP", { exact: true })).toHaveCount(0);
    await expect(page.getByText("Skills", { exact: true })).toHaveCount(0);

    const layout = await workspace.evaluate((element) => {
      const bounds = element.getBoundingClientRect();
      const rootStyle = getComputedStyle(document.documentElement);
      const bodyStyle = getComputedStyle(document.body);
      return {
        background: rootStyle.getPropertyValue("--xf-canvas").trim(),
        bodyOverflowX: bodyStyle.overflowX,
        documentOverflow: document.documentElement.scrollWidth - document.documentElement.clientWidth,
        height: Math.round(bounds.height),
        width: Math.round(bounds.width),
      };
    });

    expect(normalizeHex(layout.background)).toBe(normalizeHex(variant.expectedCanvas));
    expect(layout.height).toBeGreaterThan(variant.viewport.height * 0.8);
    expect(layout.width).toBeGreaterThan(variant.viewport.width * 0.5);
    expect(layout.documentOverflow).toBeLessThanOrEqual(0);

    const tokens = await page.evaluate(() => {
      const style = getComputedStyle(document.documentElement);
      const read = (name: string) => style.getPropertyValue(name).trim();
      return {
        canvas: read("--xf-canvas"),
        danger: read("--xf-danger"),
        faint: read("--xf-ink-faint"),
        focus: read("--xf-focus"),
        success: read("--xf-success"),
        warn: read("--xf-warn"),
      };
    });
    for (const token of ["danger", "faint", "success", "warn"] as const) {
      expect(
        contrastRatio(tokens[token], tokens.canvas),
        `${variant.name} ${token} text must retain WCAG AA contrast`,
      ).toBeGreaterThanOrEqual(4.5);
    }
    expect(
      contrastRatio(tokens.focus, tokens.canvas),
      `${variant.name} focus indicator must retain non-text contrast`,
    ).toBeGreaterThanOrEqual(3);

    if (variant.name === "narrow") {
      await expect(page.getByRole("button", { name: "Show sidebar" })).toHaveAttribute("aria-expanded", "false");
      await expect(page.getByRole("navigation", { name: "Workspace" })).not.toBeVisible();
    } else {
      await expect(page.getByRole("navigation", { name: "Workspace" })).toBeVisible();
    }

    if (variant.contrast === "more") {
      const border = await page.evaluate(() =>
        getComputedStyle(document.documentElement).getPropertyValue("--xf-border-strong").trim(),
      );
      expect(border).toBe("#252e3e");
    }

    if (variant.populatedSidebar) {
      // Chats and Projects own one column each -- only one list is ever in the
      // sidebar -- so each list is proven on the destination that owns it.
      const listBounds = () =>
        page.locator(".app-sidebar__scroll").evaluateAll((elements) =>
          elements.map((element) => ({
            clientHeight: element.clientHeight,
            scrollHeight: element.scrollHeight,
          })),
        );

      const chatBounds = await listBounds();
      expect(chatBounds).toHaveLength(1);
      expect(chatBounds.every(({ clientHeight, scrollHeight }) => scrollHeight > clientHeight)).toBe(true);

      await page.getByRole("button", { name: "Projects" }).click();
      await expect
        .poll(async () => {
          const projectBounds = await listBounds();
          return (
            projectBounds.length === 1 &&
            projectBounds.every(({ clientHeight, scrollHeight }) => scrollHeight > clientHeight)
          );
        })
        .toBe(true);

      // This variant's baseline is the chat-first column it opened on.
      await page.getByRole("button", { name: "Chats" }).click();
      await expect(page.getByRole("main", { name: "Chat workspace" })).toBeVisible();

      await expect(page.getByRole("button", { name: "Reports" })).toBeVisible();
      await expect(page.getByRole("button", { name: "Settings", exact: true })).toBeVisible();
    }

    await expect(page).toHaveScreenshot(["shell", `${variant.name}.png`]);
    await attachRenderedShell(page, testInfo, variant.name);
  });
}

test("workspace arrival preserves final shell geometry at wide and narrow widths", async ({ page }, testInfo) => {
  await installNativeBoundaryMock(page, variants[0]!, { arrivalPending: true });
  for (const viewport of [
    { name: "wide", width: 1440, height: 960 },
    { name: "narrow", width: 720, height: 900 },
  ] as const) {
    await page.setViewportSize(viewport);
    await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
    await page.goto("/");

    const skeleton = page.locator(".workspace-skeleton");
    await expect(skeleton).toBeVisible();
    await expect(skeleton).toContainText("Loading your workspace…");
    const geometry = await skeleton.evaluate((element) => {
      const sidebar = element.querySelector<HTMLElement>(".workspace-skeleton__sidebar");
      const main = element.querySelector<HTMLElement>(".workspace-skeleton__main");
      const shimmer = element.querySelector<HTMLElement>(".skeleton-shimmer");
      if (!sidebar || !main || !shimmer) throw new Error("arrival skeleton is incomplete");
      const rootBounds = element.getBoundingClientRect();
      const sidebarBounds = sidebar.getBoundingClientRect();
      const mainBounds = main.getBoundingClientRect();
      const shimmerStyle = getComputedStyle(shimmer);
      return {
        mainLeft: Math.round(mainBounds.left),
        mainWidth: Math.round(mainBounds.width),
        rootWidth: Math.round(rootBounds.width),
        sidebarDisplay: getComputedStyle(sidebar).display,
        sidebarWidth: Math.round(sidebarBounds.width),
        shimmerAnimation: shimmerStyle.animationName,
        shimmerBackgroundImage: shimmerStyle.backgroundImage,
      };
    });
    const loadingWelcome = await skeleton.evaluate((element) => {
      const composer = element.querySelector<HTMLElement>(".workspace-skeleton__composer");
      const suggestions = element.querySelector<HTMLElement>(".workspace-skeleton__suggestions");
      if (!composer || !suggestions) throw new Error("arrival welcome state is incomplete");
      const composerBounds = composer.getBoundingClientRect();
      const suggestionBounds = suggestions.getBoundingClientRect();
      return {
        composer: {
          height: Math.round(composerBounds.height),
          left: Math.round(composerBounds.left),
          top: Math.round(composerBounds.top),
          width: Math.round(composerBounds.width),
        },
        suggestionColumns: getComputedStyle(suggestions).gridTemplateColumns.split(" ").length,
        suggestions: {
          left: Math.round(suggestionBounds.left),
          width: Math.round(suggestionBounds.width),
        },
      };
    });

    expect(geometry.rootWidth).toBe(viewport.width);
    expect(geometry.shimmerAnimation).toBe("none");
    expect(geometry.shimmerBackgroundImage).toBe("none");
    if (viewport.name === "wide") {
      expect(geometry.sidebarDisplay).not.toBe("none");
      expect(geometry.sidebarWidth).toBe(304);
      expect(geometry.mainLeft).toBe(304);
      expect(geometry.mainWidth).toBe(viewport.width - 304);
    } else {
      expect(geometry.sidebarDisplay).toBe("none");
      expect(geometry.mainLeft).toBe(0);
      expect(geometry.mainWidth).toBe(viewport.width);
    }

    await attachRenderedShell(page, testInfo, `arrival-${viewport.name}`);

    await page.evaluate(() => {
      const resolve = (window as Window & { __resolveVisualWorkspaceBootstrap?: () => void })
        .__resolveVisualWorkspaceBootstrap;
      if (!resolve) throw new Error("workspace bootstrap resolver is unavailable");
      resolve();
    });
    await expect(page.getByRole("main", { name: "Chat workspace" })).toBeVisible();
    const finalComposer = page.getByRole("form", { name: "Message composer" });
    const finalSuggestions = page.getByRole("list", { name: "Suggested prompts" });
    const finalWelcome = await page.evaluate(() => {
      const composer = document.querySelector<HTMLElement>('.composer[aria-label="Message composer"]');
      const suggestions = document.querySelector<HTMLElement>('[aria-label="Suggested prompts"]');
      if (!composer || !suggestions) throw new Error("final welcome state is incomplete");
      const composerBounds = composer.getBoundingClientRect();
      const suggestionBounds = suggestions.getBoundingClientRect();
      return {
        composer: {
          height: Math.round(composerBounds.height),
          left: Math.round(composerBounds.left),
          top: Math.round(composerBounds.top),
          width: Math.round(composerBounds.width),
        },
        suggestionColumns: getComputedStyle(suggestions).gridTemplateColumns.split(" ").length,
        suggestions: {
          left: Math.round(suggestionBounds.left),
          width: Math.round(suggestionBounds.width),
        },
      };
    });

    await expect(finalComposer).toBeVisible();
    await expect(finalSuggestions).toBeVisible();
    expect(loadingWelcome.composer.width).toBe(finalWelcome.composer.width);
    expect(Math.abs(loadingWelcome.composer.height - finalWelcome.composer.height)).toBeLessThanOrEqual(1);
    expect(Math.abs(loadingWelcome.composer.left - finalWelcome.composer.left)).toBeLessThanOrEqual(1);
    expect(Math.abs(loadingWelcome.composer.top - finalWelcome.composer.top)).toBeLessThanOrEqual(2);
    expect(Math.abs(loadingWelcome.suggestions.left - finalWelcome.suggestions.left)).toBeLessThanOrEqual(1);
    expect(Math.abs(loadingWelcome.suggestions.width - finalWelcome.suggestions.width)).toBeLessThanOrEqual(1);
    expect(loadingWelcome.suggestionColumns).toBe(finalWelcome.suggestionColumns);
  }
});

test("keyboard users can reach and dismiss the shell command surfaces", async ({ page }) => {
  await openShell(page, variants[0]);

  const commandTransition = await page.getByRole("button", { name: "Search" }).evaluate((element) =>
    getComputedStyle(element)
      .transitionProperty.split(",")
      .map((property) => property.trim()),
  );
  expect(commandTransition).toEqual(
    expect.arrayContaining(["transform", "color", "background-color", "border-color", "box-shadow"]),
  );

  const sidebarToggle = page.getByRole("button", { name: "Hide sidebar" });
  const sidebarToggleGeometry = await sidebarToggle.evaluate((element) => {
    const control = getComputedStyle(element);
    const glyph = getComputedStyle(element.querySelector("svg")!);
    return {
      controlHeight: Number.parseFloat(control.height),
      controlWidth: Number.parseFloat(control.width),
      glyphHeight: Number.parseFloat(glyph.height),
      glyphWidth: Number.parseFloat(glyph.width),
      glyphStroke: Number.parseFloat(glyph.strokeWidth),
    };
  });
  expect(sidebarToggleGeometry).toEqual({
    controlHeight: 36,
    controlWidth: 36,
    glyphHeight: 16,
    glyphWidth: 16,
    glyphStroke: 1.8,
  });
  await sidebarToggle.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("button", { name: "Show sidebar" })).toBeFocused();

  await page.keyboard.press("Control+K");
  const palette = page.getByRole("dialog", { name: "Search" });
  await expect(palette).toBeVisible();
  await expect(page.getByRole("combobox", { name: "Search workspace" })).toBeFocused();
  const reducedMotionDurations = await palette.evaluate((element) => {
    const style = getComputedStyle(element);
    const toMilliseconds = (value: string) =>
      value.endsWith("ms") ? Number.parseFloat(value) : Number.parseFloat(value) * 1000;
    return {
      animation: toMilliseconds(style.animationDuration),
      transition: toMilliseconds(style.transitionDuration),
    };
  });
  expect(reducedMotionDurations.animation).toBeLessThanOrEqual(0.1);
  expect(reducedMotionDurations.transition).toBeLessThanOrEqual(0.1);
  await page.keyboard.press("Escape");
  await expect(palette).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Show sidebar" })).toBeFocused();
});

test("shell chrome renders on the named type ramp and nothing below it", async ({ page }) => {
  await openShell(page, variants[0]);

  // Probed through the cascade, not read off the custom properties:
  // `calc(.875rem * var(--xf-type-scale))` comes back from getComputedStyle
  // unresolved, and it is the rendered px that has to be ordered and shared.
  const ramp = await page.evaluate(() => {
    const roles = ["micro", "caption", "label", "body", "heading", "title", "display"];
    // One probe per role. Re-pointing a single element's font-size at each
    // variable in turn reads the first value back seven times -- the style
    // recalc does not land between the write and the read.
    const probes = roles.map((role) => {
      const probe = document.createElement("span");
      probe.style.position = "absolute";
      probe.style.visibility = "hidden";
      probe.style.fontSize = `var(--xf-type-${role})`;
      document.body.append(probe);
      return probe;
    });
    const sizes: Record<string, number> = {};
    roles.forEach((role, index) => {
      sizes[role] = Number.parseFloat(getComputedStyle(probes[index]).fontSize);
    });
    for (const probe of probes) probe.remove();
    return sizes;
  });

  // Each role is a distinct ascending step. A duplicated or inverted step means
  // a role has stopped being a level in the hierarchy.
  const ascending = ["micro", "caption", "label", "body", "heading", "title", "display"];
  expect(ramp.micro).toBeGreaterThan(0);
  for (let index = 1; index < ascending.length; index += 1) {
    expect(ramp[ascending[index]]).toBeGreaterThan(ramp[ascending[index - 1]]);
  }

  const fontSizeOf = (selector: string) =>
    page
      .locator(selector)
      .first()
      .evaluate((element) => Number.parseFloat(getComputedStyle(element).fontSize));

  // Two sizes per screen (xfalcon-design DESIGN.md 2.3): every piece of chrome,
  // the destinations and list rows included, reads at the one chrome size
  // (12px at the default reading size), on or above the ramp's floor.
  const chrome = await page.evaluate(() => {
    const probe = document.createElement("span");
    probe.style.fontSize = "var(--xf-type-chrome)";
    document.body.append(probe);
    const size = Number.parseFloat(getComputedStyle(probe).fontSize);
    probe.remove();
    return size;
  });
  expect(chrome).toBeGreaterThanOrEqual(ramp.micro);
  for (const selector of [
    ".app-command-button kbd",
    ".nav-destination",
    ".app-sidebar__content .nav-item",
    ".app-titlebar strong",
  ]) {
    expect(await fontSizeOf(selector)).toBeCloseTo(chrome, 1);
  }

  // The floor proof: no visible text in the shell renders under `micro`, so a
  // reintroduced 9px or 10px one-off fails here rather than at review.
  const belowRamp = await page.evaluate((floor) => {
    const offenders: string[] = [];
    for (const element of document.body.querySelectorAll<HTMLElement>("*")) {
      const hasOwnText = Array.from(element.childNodes).some(
        (node) => node.nodeType === Node.TEXT_NODE && (node.textContent ?? "").trim().length > 0,
      );
      if (!hasOwnText || element.offsetParent === null) continue;
      const size = Number.parseFloat(getComputedStyle(element).fontSize);
      if (size + 0.1 < floor) offenders.push(`${element.tagName.toLowerCase()}.${element.className} @ ${size}px`);
    }
    return offenders;
  }, ramp.micro);
  expect(belowRamp).toEqual([]);
});

test("forced-colors mode keeps native focus and control boundaries available", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light", forcedColors: "active", reducedMotion: "reduce" });
  await installNativeBoundaryMock(page, variants[0]);
  await page.goto("/");
  await expect(page.getByRole("main", { name: "Chat workspace" })).toBeVisible();

  expect(await page.evaluate(() => matchMedia("(forced-colors: active)").matches)).toBe(true);
  const focusOutline = await page.getByRole("button", { name: "Hide sidebar" }).evaluate((element) => {
    element.focus();
    const style = getComputedStyle(element);
    return { color: style.outlineColor, style: style.outlineStyle, width: style.outlineWidth };
  });
  expect(focusOutline.style).not.toBe("none");
  expect(Number.parseFloat(focusOutline.width)).toBeGreaterThanOrEqual(2);
});

/**
 * One focus treatment, and it may not reshape the control it lands on.
 *
 * The global rule in `index.css` gives every focusable element the same
 * outline. A primitive that re-declares focus in Tailwind is a second answer to
 * the same question, and the ones that did had drifted onto three different
 * colours.
 */
test("keyboard focus is one treatment across the primitives", async ({ page }) => {
  await openShell(page, variants[0]!);

  const focusToken = await page.evaluate(() =>
    getComputedStyle(document.documentElement).getPropertyValue("--xf-focus").trim(),
  );

  const composer = page.getByRole("textbox", { name: "Message xFalcon" });
  await composer.fill("hello");

  const probes = [
    page.getByRole("button", { name: "Send message" }),
    page.getByRole("button", { name: "Attach files" }),
    page.getByRole("button", { name: "Search" }),
    page.getByRole("button", { name: "Hide sidebar" }),
    page.getByRole("button", { name: "New chat" }),
  ];

  for (const probe of probes) {
    const resting = await probe.evaluate((element) => getComputedStyle(element).borderRadius);
    await probe.focus();
    const focused = await probe.evaluate((element) => ({
      radius: getComputedStyle(element).borderRadius,
      outlineColor: getComputedStyle(element).outlineColor,
      outlineStyle: getComputedStyle(element).outlineStyle,
      outlineWidth: getComputedStyle(element).outlineWidth,
    }));

    // A circular control must not square off when it takes focus.
    expect(focused.radius, `${await probe.getAttribute("aria-label")} radius`).toBe(resting);
    expect(focused.outlineStyle, `${await probe.getAttribute("aria-label")} outline`).toBe("solid");
    expect(focused.outlineWidth).toBe("2px");
    expect(toRgb(focusToken)).toBe(focused.outlineColor);
  }
});

/**
 * The destination well holds four rows whose height scales with the reading
 * size. The well must grow with those rows, while the approved flat selection
 * stays flush with its edge at every setting.
 */
test("the destination group fits its rows and keeps the selection flush at every reading size", async ({ page }) => {
  await openShell(page, variants[0]!);

  const measurements = await page.evaluate(() => {
    const readings: Record<string, { padding: number; rowGrowth: number; wellGrowth: number; selectionTop: number }> =
      {};
    const well = document.querySelector<HTMLElement>(".app-sidebar__destinations");
    if (!well) throw new Error("the destination group is not in the sidebar");

    const measure = () => {
      // The selection highlight is absolutely positioned. It changes with the
      // row token but does not participate in the well's layout.
      const rows = Array.from(well.children)
        .filter((row) => getComputedStyle(row).position !== "absolute")
        .map((row) => row.getBoundingClientRect().height);
      return { rows: rows.reduce((total, height) => total + height, 0), well: well.getBoundingClientRect().height };
    };

    document.documentElement.setAttribute("data-font-size", "medium");
    const base = measure();
    for (const size of ["small", "medium", "large"]) {
      document.documentElement.setAttribute("data-font-size", size);
      const now = measure();
      readings[size] = {
        padding: Number.parseFloat(getComputedStyle(well).paddingTop),
        rowGrowth: now.rows - base.rows,
        wellGrowth: now.well - base.well,
        selectionTop:
          well.querySelector(".nav-destination__highlight")!.getBoundingClientRect().top -
          well.getBoundingClientRect().top,
      };
    }
    document.documentElement.setAttribute("data-font-size", "medium");
    return readings;
  });

  for (const [size, reading] of Object.entries(measurements)) {
    // The well has to grow by at least what its rows grew by. If it grows by
    // less, the larger type is being clipped or crowded.
    expect(reading.wellGrowth + 0.5, `${size} well vs rows`).toBeGreaterThanOrEqual(reading.rowGrowth);
    // The approved flat selection meets the well's edge; an inset would
    // reintroduce the corner leak removed by d736011.
    expect(reading.padding, `${size} padding`).toBe(0);
    expect(Math.abs(reading.selectionTop), `${size} selection top`).toBeLessThan(0.5);
  }
});

/**
 * One label column for the sidebar's icon rows.
 *
 * `--xf-nav-label-x` declared the column and nothing used it, so the column was
 * whatever each row's glyph happened to leave: a 1.125rem Settings icon put
 * that one label 2px right of every other. The glyph sits in a fixed
 * `--xf-nav-icon-col` box now, so changing a glyph moves the glyph.
 *
 * A row WITHOUT an icon is deliberately a different column -- a chat title
 * steps in by `--xf-nav-indent`, the same step a project's files take under
 * their project -- so this measures the icon rows and says so.
 */
test("the sidebar keeps one label column for its icon rows", async ({ page }) => {
  await openShell(
    page,
    variants.find((variant) => variant.name === "populated-sidebar")!,
  );

  const columns = await page.evaluate(() => {
    const sidebar = document.querySelector<HTMLElement>(".app-sidebar")!;
    const sidebarLeft = sidebar.getBoundingClientRect().left;
    const firstText = (node: Element): Text | null => {
      for (const child of node.childNodes) {
        if (child.nodeType === Node.TEXT_NODE && child.textContent?.trim()) return child as Text;
        if (child.nodeType === Node.ELEMENT_NODE) {
          const found = firstText(child as Element);
          if (found) return found;
        }
      }
      return null;
    };

    const found: { row: string; destination: boolean; labelX: number; iconWidth: number }[] = [];
    for (const row of sidebar.querySelectorAll<HTMLElement>(".nav-destination, .nav-item")) {
      const icon = row.querySelector("svg");
      const text = firstText(row);
      if (!icon || !text || row.getBoundingClientRect().height === 0) continue;
      const range = document.createRange();
      range.selectNodeContents(text);
      found.push({
        destination: row.classList.contains("nav-destination"),
        iconWidth: Math.round(icon.getBoundingClientRect().width),
        labelX: Math.round(range.getBoundingClientRect().left - sidebarLeft),
        row: text.textContent!.trim().slice(0, 24),
      });
    }
    return found;
  });

  expect(columns.length).toBeGreaterThanOrEqual(3);
  // Two columns, each held exactly: the list rows share one, and the
  // destinations share one a fixed step right of it -- the clearance that keeps
  // the current destination's selection bar off its icon
  // (--xf-destination-marker-clearance).
  const MARKER_CLEARANCE_PX = 9;
  const destinations = columns.filter((row) => row.destination);
  const listRows = columns.filter((row) => !row.destination);
  expect(destinations.length).toBeGreaterThanOrEqual(1);
  expect(listRows.length).toBeGreaterThanOrEqual(1);
  for (const group of [destinations, listRows]) {
    const [first, ...rest] = group;
    for (const row of rest) {
      expect(row.labelX, `"${row.row}" label column against "${first.row}"`).toBe(first.labelX);
    }
  }
  expect(destinations[0].labelX, "destination column against the list column").toBe(
    listRows[0].labelX + MARKER_CLEARANCE_PX,
  );
  // And the column holds because the glyph box is fixed, not because every
  // caller happens to have picked the same size today.
  for (const row of columns) expect(row.iconWidth, `"${row.row}" glyph box`).toBe(16);
});

/**
 * A control that can be clicked says so with the cursor.
 *
 * Tailwind v4's preflight matches the browser and leaves a `<button>` on the
 * default arrow, so the pointer appeared only where a caller had remembered
 * `cursor-pointer`. 38 of the 71 live controls in this shell had not --
 * including every Pin and Delete action on every chat row, which are the only
 * way to reach those commands with a mouse.
 */
test("every live control in the shell offers the pointer", async ({ page }) => {
  await openShell(
    page,
    variants.find((variant) => variant.name === "populated-sidebar")!,
  );

  const cursors = await page.evaluate(() => {
    const offenders: string[] = [];
    let checked = 0;
    for (const element of document.querySelectorAll<HTMLElement>("button, [role='button'], a[href]")) {
      if (element.getBoundingClientRect().height === 0) continue;
      if ((element as HTMLButtonElement).disabled) continue;
      if (element.getAttribute("aria-disabled") === "true") continue;
      checked += 1;
      const cursor = getComputedStyle(element).cursor;
      if (cursor !== "pointer") {
        offenders.push(
          `${cursor} :: ${(element.getAttribute("aria-label") ?? element.textContent ?? "").trim().slice(0, 32)}`,
        );
      }
    }
    return { checked, offenders };
  });

  expect(cursors.checked).toBeGreaterThan(40);
  expect(cursors.offenders).toEqual([]);
});

for (const theme of ["light", "dark"] as const) {
  test(`the collapsed sidebar rail preserves navigation at ${theme}`, async ({ page }) => {
    const variant = variants.find((entry) => entry.name === theme)!;
    await openShell(page, variant);
    await page.getByRole("button", { name: "Hide sidebar" }).click();
    const rail = page.getByRole("navigation", { name: "Workspace" });
    await expect(rail).toBeVisible();
    await expect(page.locator('aside.app-sidebar[data-rail="true"]')).toBeVisible();
    await expect(page).toHaveScreenshot(["shell", `rail-${theme}.png`]);
  });
}

test("the narrow drawer preserves the populated sidebar overlay", async ({ page }) => {
  const variant = variants.find((entry) => entry.name === "populated-sidebar")!;
  await page.setViewportSize({ width: 720, height: 900 });
  await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
  await installNativeBoundaryMock(page, { ...variant, viewport: { width: 720, height: 900 } });
  await page.goto("/");
  await expect(page.getByRole("main", { name: "Chat workspace" })).toBeVisible();
  await page.evaluate(async () => document.fonts.ready);
  await page.getByRole("button", { name: "Show sidebar" }).click();
  await expect(page.getByRole("navigation", { name: "Workspace" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Close sidebar" })).toBeVisible();
  await expect(page).toHaveScreenshot(["shell", "drawer-narrow-light.png"]);
});

test("the populated projects list preserves the sidebar scroll contract", async ({ page }) => {
  const variant = variants.find((entry) => entry.name === "populated-sidebar")!;
  await openShell(page, variant);
  await page.getByRole("button", { name: "Projects" }).click();
  const startProjectChat = page.getByRole("button", { name: "New chat in Research project 1", exact: true });
  await expect(page.getByRole("treeitem").filter({ has: startProjectChat })).toBeVisible();
  await expect(startProjectChat).toBeVisible();
  await expect(page).toHaveScreenshot(["shell", "populated-projects-light.png"]);
});

for (const theme of ["light", "dark"] as const) {
  test(`the open command palette preserves the shared surface at ${theme}`, async ({ page }) => {
    const variant = variants.find((entry) => entry.name === theme)!;
    await openShell(page, variant);
    await page.keyboard.press("Control+K");
    const palette = page.getByRole("dialog", { name: "Search" });
    await expect(palette).toBeVisible();
    await expect(page.getByRole("combobox", { name: "Search workspace" })).toBeFocused();
    await expect(page).toHaveScreenshot(["shell", `palette-open-${theme}.png`]);
  });
}

test("the command palette preserves a large result catalog", async ({ page }) => {
  const variant = variants.find((entry) => entry.name === "populated-sidebar")!;
  await openShell(page, variant);
  await page.keyboard.press("Control+K");
  await page.getByRole("combobox", { name: "Search workspace" }).fill("research");
  await expect(page.getByRole("option", { name: /Research project/ }).first()).toBeVisible();
  await expect(page).toHaveScreenshot(["shell", "palette-catalog-light.png"]);
});

test("the command palette preserves narrow placement", async ({ page }) => {
  await page.setViewportSize({ width: 720, height: 900 });
  await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
  await installVisualNativeMock(page, { theme: "light", populatedSidebar: true });
  await page.goto("/");
  await expect(page.getByRole("main", { name: "Chat workspace" })).toBeVisible();
  await page.evaluate(async () => document.fonts.ready);
  await page.keyboard.press("Control+K");
  await expect(page.getByRole("dialog", { name: "Search" })).toBeVisible();
  await expect(page).toHaveScreenshot(["shell", "palette-narrow-light.png"]);
});

for (const theme of ["light", "dark"] as const) {
  test(`the open account menu preserves identity and session actions at ${theme}`, async ({ page }) => {
    const variant = variants.find((entry) => entry.name === theme)!;
    await openShell(page, variant);
    await page.getByRole("button", { name: "Account and app menu" }).click();
    await expect(page.getByRole("menuitem", { name: "Sign out" })).toBeVisible();
    await expect(page).toHaveScreenshot(["shell", `account-menu-${theme}.png`]);
  });
}

test("live resize keeps the desktop rail choice separate from the narrow drawer", async ({ page }) => {
  const variant = variants.find((entry) => entry.name === "light")!;
  await openShell(page, variant);
  const shell = page.locator(".app-shell");

  await page.setViewportSize({ width: 900, height: 800 });
  await expect(shell).toHaveAttribute("data-sidebar-open", "false");
  await expect(page.locator('aside.app-sidebar[data-rail="true"]')).toHaveCount(0);

  await page.setViewportSize({ width: 1440, height: 960 });
  await expect(shell).toHaveAttribute("data-sidebar-open", "true");
  await expect(page.locator('aside.app-sidebar[data-rail="true"]')).toHaveCount(0);

  await page.keyboard.press("Control+B");
  await expect(page.locator('aside.app-sidebar[data-rail="true"]')).toBeVisible();
  await page.setViewportSize({ width: 900, height: 800 });
  await expect(shell).toHaveAttribute("data-sidebar-open", "false");
  await page.setViewportSize({ width: 1440, height: 960 });
  await expect(page.locator('aside.app-sidebar[data-rail="true"]')).toBeVisible();
});

test("keyboard-only navigation reaches the rail and its shared help", async ({ page }) => {
  const variant = variants.find((entry) => entry.name === "light")!;
  await openShell(page, variant);
  await page.keyboard.press("Control+B");

  const chats = page.getByRole("button", { name: "Chats" });
  for (let step = 0; step < 12 && !(await chats.evaluate((element) => document.activeElement === element)); step += 1) {
    await page.keyboard.press("Tab");
  }
  await expect(chats).toBeFocused();
  await expect(page.getByRole("tooltip")).toHaveText("Chats");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("tooltip")).toHaveCount(0);
  await page.keyboard.press("Control+B");
  await expect(page.getByRole("navigation", { name: "Workspace" })).toBeVisible();
});

test.describe("coarse pointer shell", () => {
  test.use({ hasTouch: true });

  test("gives every rail control a 44px touch target", async ({ page }) => {
    const variant = variants.find((entry) => entry.name === "light")!;
    await openShell(page, variant);
    expect(await page.evaluate(() => matchMedia("(pointer: coarse)").matches)).toBe(true);
    await page.keyboard.press("Control+B");

    const undersized = await page
      .locator('aside.app-sidebar[data-rail="true"] button:not([disabled])')
      .evaluateAll((buttons) =>
        buttons.flatMap((button) => {
          const bounds = button.getBoundingClientRect();
          return bounds.width >= 44 && bounds.height >= 44
            ? []
            : [
                `${button.getAttribute("aria-label") ?? "unnamed"}: ${Math.round(bounds.width)}x${Math.round(bounds.height)}`,
              ];
        }),
      );
    expect(undersized).toEqual([]);
  });
});

test("the 900px native minimum holds long labels and 200% text without horizontal loss", async ({ page }) => {
  const nativeConfig = JSON.parse(
    readFileSync(new URL("../../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
  ) as {
    app: { windows: Array<{ minWidth: number }> };
  };
  expect(nativeConfig.app.windows[0]?.minWidth).toBe(900);

  await page.setViewportSize({ width: 900, height: 800 });
  await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
  await installVisualNativeMock(page, { theme: "light", populatedSidebar: true, longLabels: true });
  await page.goto("/");
  await expect(page.getByRole("main", { name: "Chat workspace" })).toBeVisible();
  await page.keyboard.press("Control+B");
  await expect(page.getByRole("navigation", { name: "Workspace" })).toBeVisible();
  await expect(page.locator(".app-account-trigger strong")).toHaveText(
    /^Quarterly revenue and customer-retention analysis/,
  );
  await page.evaluate(async () => {
    await document.fonts.ready;
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    document.documentElement.style.fontSize = "32px";
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
  });
  await expect.poll(() => page.evaluate(() => getComputedStyle(document.documentElement).fontSize)).toBe("32px");

  const layout = await page.evaluate(() => {
    const accountName = document.querySelector<HTMLElement>(".app-account-trigger strong");
    const sidebar = document.querySelector<HTMLElement>(".app-sidebar");
    if (!accountName || !sidebar) throw new Error("long-label fixture did not render the shell controls");
    const contentOverflow = [...sidebar.querySelectorAll<HTMLElement>("*")].flatMap((element) => {
      const style = getComputedStyle(element);
      if (style.display === "none" || style.textOverflow === "ellipsis" || element.classList.contains("sr-only"))
        return [];
      return element.scrollWidth > element.clientWidth + 2 && style.overflowX === "hidden"
        ? [element.className.toString().split(/\s+/)[0] || element.tagName]
        : [];
    });
    return {
      accountTruncates: accountName.scrollWidth > accountName.clientWidth,
      documentOverflow: document.documentElement.scrollWidth - document.documentElement.clientWidth,
      hiddenOverflow: [...new Set(contentOverflow)],
    };
  });

  expect(layout.accountTruncates).toBe(true);
  expect(layout.documentOverflow).toBeLessThanOrEqual(0);
  expect(layout.hiddenOverflow).toEqual([]);
});

/** `#rrggbb` as the `rgb(r, g, b)` a computed style reports. */
function toRgb(hex: string): string {
  const value = hex.replace("#", "");
  const [r, g, b] = [0, 2, 4].map((offset) => parseInt(value.slice(offset, offset + 2), 16));
  return `rgb(${r}, ${g}, ${b})`;
}
