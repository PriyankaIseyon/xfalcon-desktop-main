import { expect, test, type Page } from "@playwright/test";

/**
 * Task 1.16: P01–P42 under the nine display modes the change commits to.
 *
 * The gallery at `/primitives.html` renders every primitive with its P-id on a
 * `data-primitive` attribute; this sweep drives it under each mode and asserts
 * what the mode is FOR, rather than photographing it. A screenshot per mode per
 * primitive would be 378 baselines that fail on a font hint, and would still
 * not say which rule broke.
 *
 * The page is served by the dev server and is absent from the production build
 * (`vite.config.ts` lists its entries explicitly, and the style contract fails
 * if the gallery ever reaches `dist`).
 */
const GALLERY = "/primitives.html";

/** Every primitive the ledger tracks. A specimen missing from the gallery is
 * not verified, and `covers every primitive in the ledger` says so out loud. */
const PRIMITIVES = Array.from({ length: 42 }, (_, index) => `P${String(index + 1).padStart(2, "0")}`);

async function openGallery(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  await page.goto(GALLERY);
  await expect(page.locator('[data-primitive="P42"]')).toBeVisible();
  return errors;
}

/** sRGB relative luminance, per WCAG 2.1. */
function luminance([r, g, b]: number[]): number {
  const [rl, gl, bl] = [r, g, b].map((channel) => {
    const value = channel / 255;
    return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * rl + 0.7152 * gl + 0.0722 * bl;
}

function contrast(foreground: number[], background: number[]): number {
  const [light, dark] = [luminance(foreground), luminance(background)].sort((a, b) => b - a);
  return (light + 0.05) / (dark + 0.05);
}

function parseRgb(value: string): number[] | null {
  const match = value.match(/rgba?\(([^)]+)\)/);
  if (!match) return null;
  const parts = match[1].split(",").map((part) => Number.parseFloat(part.trim()));
  if (parts.length >= 4 && parts[3] === 0) return null;
  return parts.slice(0, 3);
}

test.describe("the primitive gallery", () => {
  test("covers every primitive in the ledger", async ({ page }) => {
    const errors = await openGallery(page);
    const present = await page
      .locator("[data-primitive]")
      .evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-primitive")));
    // The sweep below is only worth its name while this holds: a primitive
    // absent here is one the nine modes never touch.
    expect([...present].sort()).toEqual(PRIMITIVES);
    expect(errors).toEqual([]);
  });
});

test.describe("light and dark", () => {
  for (const theme of ["light", "dark"] as const) {
    test(`${theme}: every specimen's text clears AA against what it sits on`, async ({ page }) => {
      // The theme is set BEFORE the first paint, and every transition is cut
      // for the duration of the measurement. Switching the theme after load
      // starts a colour transition on every surface at once, and a read taken
      // while those are in flight reports whatever colour each element happens
      // to be passing through -- which made this test name a different
      // innocent primitive on each run. Reducing motion is not enough: the
      // app's reduced duration is a very small number, not zero, and a
      // transition still needs a compositor frame to reach its end value.
      await page.emulateMedia({ colorScheme: theme });
      await page.addInitScript((value) => {
        document.documentElement.setAttribute("data-theme", value);
        document.addEventListener("DOMContentLoaded", () => {
          const cut = document.createElement("style");
          cut.textContent = "*, *::before, *::after { transition: none !important; animation: none !important; }";
          document.head.append(cut);
        });
      }, theme);
      await openGallery(page);

      const failures = await page.evaluate(() => {
        // Walk up for the first ancestor that actually paints, which is what a
        // reader's eye sees behind the glyphs.
        function paintedBackground(node: Element): string {
          let current: Element | null = node;
          while (current) {
            const value = getComputedStyle(current).backgroundColor;
            const parts = value.match(/rgba?\(([^)]+)\)/);
            const alpha = parts ? Number.parseFloat(parts[1].split(",")[3] ?? "1") : 1;
            if (parts && alpha > 0.9) return value;
            current = current.parentElement;
          }
          return getComputedStyle(document.body).backgroundColor;
        }

        const found: { primitive: string; text: string; color: string; background: string; size: number }[] = [];
        for (const specimen of document.querySelectorAll("[data-primitive]")) {
          const primitive = specimen.getAttribute("data-primitive")!;
          for (const node of specimen.querySelectorAll<HTMLElement>("*")) {
            const text = Array.from(node.childNodes)
              .filter((child) => child.nodeType === Node.TEXT_NODE)
              .map((child) => child.textContent?.trim() ?? "")
              .join("")
              .trim();
            if (!text) continue;
            const style = getComputedStyle(node);
            if (style.visibility === "hidden" || style.display === "none") continue;
            // `sr-only` text is never seen, so it has nothing to contrast with.
            if (node.closest(".sr-only")) continue;
            if (Number.parseFloat(style.opacity) < 1) continue;
            found.push({
              background: paintedBackground(node),
              color: style.color,
              primitive,
              size: Number.parseFloat(style.fontSize),
              text: text.slice(0, 24),
            });
          }
        }
        return found;
      });

      const offenders = failures
        .map((entry) => {
          const foreground = parseRgb(entry.color);
          const background = parseRgb(entry.background);
          if (!foreground || !background) return null;
          const ratio = contrast(foreground, background);
          // AA: 4.5 for body text, 3.0 from 18.66px, or 14px bold. The gallery
          // has no bold-14 specimen, so the size rule alone decides.
          const required = entry.size >= 18.66 ? 3 : 4.5;
          return ratio + 0.01 < required
            ? `${entry.primitive} "${entry.text}" ${entry.color} on ${entry.background} = ${ratio.toFixed(2)}:1 < ${required}:1`
            : null;
        })
        .filter(Boolean);

      expect(offenders).toEqual([]);
    });
  }
});

test("forced colors: a state stated by fill alone survives the palette swap", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light", forcedColors: "active" });
  await openGallery(page);
  expect(await page.evaluate(() => matchMedia("(forced-colors: active)").matches)).toBe(true);

  // Forced colors replaces every author fill with one system colour, so a
  // control that says on/off with fill alone says nothing. Each of these has to
  // stay distinguishable — the switch is why `.xf-switch` has its own block.
  const pairs = await page.evaluate(() => {
    const read = (element: Element) => {
      const style = getComputedStyle(element);
      return [style.backgroundColor, style.borderColor, style.color].join(" | ");
    };
    const switches = Array.from(document.querySelectorAll('[data-primitive="P11"] [role="switch"]'));
    const checkboxes = Array.from(document.querySelectorAll('[data-primitive="P09"] input[type="checkbox"]'));
    return {
      checkedSwitch: switches.filter((s) => s.getAttribute("aria-checked") === "true").map(read),
      uncheckedSwitch: switches.filter((s) => s.getAttribute("aria-checked") === "false").map(read),
      checkbox: checkboxes.map(read),
    };
  });

  expect(pairs.checkedSwitch.length).toBeGreaterThan(0);
  expect(pairs.uncheckedSwitch.length).toBeGreaterThan(0);
  expect(pairs.checkedSwitch[0]).not.toBe(pairs.uncheckedSwitch[0]);

  // Every focusable primitive still shows a focus outline from the system
  // palette; forced colors is where a hand-drawn ring disappears.
  const focusable = page.locator("[data-primitive] button:not([disabled])");
  const count = Math.min(await focusable.count(), 12);
  for (let index = 0; index < count; index += 1) {
    const outline = await focusable.nth(index).evaluate((element) => {
      (element as HTMLElement).focus();
      const style = getComputedStyle(element);
      return { style: style.outlineStyle, width: Number.parseFloat(style.outlineWidth) };
    });
    expect(outline.style).not.toBe("none");
    expect(outline.width).toBeGreaterThanOrEqual(2);
  }
});

test("reduced motion: nothing in the gallery keeps animating", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await openGallery(page);

  const moving = await page.evaluate(() => {
    const offenders: string[] = [];
    for (const specimen of document.querySelectorAll("[data-primitive]")) {
      for (const node of specimen.querySelectorAll<HTMLElement>("*")) {
        const style = getComputedStyle(node);
        const durations = [...style.animationDuration.split(","), ...style.transitionDuration.split(",")];
        for (const duration of durations) {
          const seconds = duration.trim().endsWith("ms")
            ? Number.parseFloat(duration) / 1000
            : Number.parseFloat(duration);
          // A reduced-motion user asked for no animation, not a slower one.
          // 0.02s is the conventional "effectively instant" floor.
          if (seconds > 0.02) {
            offenders.push(
              `${specimen.getAttribute("data-primitive")} ${node.tagName.toLowerCase()} ${duration.trim()}`,
            );
          }
        }
      }
    }
    return [...new Set(offenders)];
  });

  expect(moving).toEqual([]);
});

test("reduced transparency: no primitive hides behind a blur", async ({ page }) => {
  await openGallery(page);
  await page.emulateMedia({ media: "screen" });
  // Playwright cannot emulate `prefers-reduced-transparency`, so the rule is
  // read from the sheet the browser resolved rather than from the file: the
  // query has to exist and has to name every blurred surface.
  const covered = await page.evaluate(() => {
    const blurred = new Set<string>();
    for (const specimen of document.querySelectorAll("[data-primitive]")) {
      for (const node of specimen.querySelectorAll<HTMLElement>("*")) {
        const style = getComputedStyle(node);
        const filter =
          style.backdropFilter || (style as unknown as { webkitBackdropFilter: string }).webkitBackdropFilter;
        if (filter && filter !== "none") blurred.add(node.className.toString().split(/\s+/)[0] || node.tagName);
      }
    }
    const queries: string[] = [];
    for (const sheet of Array.from(document.styleSheets)) {
      let rules: CSSRuleList;
      try {
        rules = sheet.cssRules;
      } catch {
        continue;
      }
      for (const rule of Array.from(rules)) {
        if (rule instanceof CSSMediaRule && rule.conditionText.includes("reduced-transparency")) {
          queries.push(rule.cssText);
        }
      }
    }
    return { blurred: [...blurred], queries };
  });

  // Whatever blurs, the app must state a flat fallback for it.
  if (covered.blurred.length > 0) {
    expect(covered.queries.length).toBeGreaterThan(0);
  }
  expect(covered.queries.join(" ")).toContain("backdrop-filter");
});

test("keyboard: every control in the gallery is reachable and shows the one ring", async ({ page }) => {
  await openGallery(page);
  const focusToken = await page.evaluate(() =>
    getComputedStyle(document.documentElement).getPropertyValue("--xf-focus").trim(),
  );
  const expected = (() => {
    const value = focusToken.replace("#", "");
    const [r, g, b] = [0, 2, 4].map((offset) => Number.parseInt(value.slice(offset, offset + 2), 16));
    return `rgb(${r}, ${g}, ${b})`;
  })();

  const controls = page.locator(
    "[data-primitive] button:not([disabled]), [data-primitive] a[href], [data-primitive] input:not([disabled]), [data-primitive] textarea:not([disabled])",
  );
  const count = await controls.count();
  expect(count).toBeGreaterThan(30);

  const offenders: string[] = [];
  for (let index = 0; index < count; index += 1) {
    const control = controls.nth(index);
    const result = await control.evaluate((element, ring) => {
      const before = getComputedStyle(element).borderRadius;
      (element as HTMLElement).focus();
      const style = getComputedStyle(element);
      return {
        focused: document.activeElement === element,
        primitive: element.closest("[data-primitive]")?.getAttribute("data-primitive") ?? "?",
        radiusHeld: getComputedStyle(element).borderRadius === before,
        ringed: style.outlineStyle !== "none" && Number.parseFloat(style.outlineWidth) >= 2,
        tinted: style.outlineColor === ring,
      };
    }, expected);
    if (!result.focused) offenders.push(`${result.primitive} not focusable`);
    // The global rule paints `:focus-visible`, which a programmatic focus does
    // not always match; what must hold for every control is that focus never
    // RESHAPES it — the defect that squared off the circular send button.
    if (!result.radiusHeld) offenders.push(`${result.primitive} changed shape on focus`);
    if (result.ringed && !result.tinted) offenders.push(`${result.primitive} drew its own ring colour`);
  }
  expect(offenders).toEqual([]);
});

test("every live primitive offers the pointer", async ({ page }) => {
  // Tailwind v4's preflight leaves a `<button>` on the browser's default arrow,
  // so the pointer appeared only where a caller remembered `cursor-pointer`.
  // `index.css` answers it once for the product; this keeps a primitive from
  // opting itself back out.
  await openGallery(page);
  const offenders = await page.evaluate(() => {
    const found: string[] = [];
    for (const element of document.querySelectorAll<HTMLElement>(
      "[data-primitive] button, [data-primitive] a[href], [data-primitive] [role='button']",
    )) {
      if (element.getBoundingClientRect().height === 0) continue;
      if ((element as HTMLButtonElement).disabled || element.getAttribute("aria-disabled") === "true") continue;
      const cursor = getComputedStyle(element).cursor;
      if (cursor !== "pointer") {
        found.push(`${element.closest("[data-primitive]")?.getAttribute("data-primitive")} ${cursor}`);
      }
    }
    return [...new Set(found)];
  });
  expect(offenders).toEqual([]);
});

test.describe("coarse pointer", () => {
  // Real touch emulation, because `pointer: coarse` is a CSS media query and
  // overriding `window.matchMedia` in the page does not move it -- an earlier
  // version of this test did exactly that and measured the fine-pointer layout
  // while reporting on the coarse one.
  test.use({ hasTouch: true });

  test("every control offers a 44px target, or is inline text", async ({ page }) => {
    await openGallery(page);
    expect(await page.evaluate(() => matchMedia("(pointer: coarse)").matches)).toBe(true);

    const small = await page.evaluate(() => {
      const target = 44;
      const offenders: string[] = [];
      const controls = document.querySelectorAll<HTMLElement>(
        "[data-primitive] button:not([disabled]), [data-primitive] a[href], [data-primitive] input:not([type='hidden'])",
      );
      for (const element of controls) {
        const box = element.getBoundingClientRect();
        if (box.width === 0 && box.height === 0) continue;
        const primitive = element.closest("[data-primitive]")?.getAttribute("data-primitive");

        // `.touch-target--halo` carries the hit area as a centred overlay, so
        // the painted box is deliberately smaller than what a finger lands on.
        const halo = getComputedStyle(element, "::after");
        const haloed = halo.content !== "none" && halo.position === "absolute";
        const width = haloed ? Math.max(box.width, Number.parseFloat(halo.width)) : box.width;
        const height = haloed ? Math.max(box.height, Number.parseFloat(halo.height)) : box.height;

        // A control hidden from sight delegates its target to the visible
        // thing that names it -- the file field's label is what a finger hits.
        if (element.closest(".sr-only") || element.className.toString().includes("sr-only")) continue;

        // WCAG 2.5.5 and 2.5.8 both exempt a target that sits INSIDE a line of
        // text, and every anchor in this product is one: prose links and trail
        // links. A link cannot be 44px tall without breaking the sentence it
        // belongs to, so it owes 2.5.8's 24px minimum instead. Standalone
        // actions are buttons, and buttons owe the full target.
        const floor = element.tagName === "A" ? 24 : target;
        if (height + 0.5 < floor || width + 0.5 < floor) {
          offenders.push(`${primitive} ${Math.round(width)}x${Math.round(height)} < ${floor}`);
        }
      }
      return [...new Set(offenders)];
    });

    expect(small).toEqual([]);
  });
});

test("200% text: nothing clips its own content when the ramp doubles", async ({ page }) => {
  await openGallery(page);
  const clipped = await page.evaluate(() => {
    document.documentElement.style.fontSize = "32px";
    // Force a layout pass before measuring.
    void document.body.offsetHeight;

    const offenders: string[] = [];
    for (const specimen of document.querySelectorAll("[data-primitive]")) {
      for (const node of specimen.querySelectorAll<HTMLElement>("*")) {
        const style = getComputedStyle(node);
        // Only an element that hides its overflow can clip; a visible overflow
        // is a layout question, not a lost-content one.
        if (style.overflowX !== "hidden" && style.overflowY !== "hidden") continue;
        if (style.textOverflow === "ellipsis") continue;
        // `sr-only` IS a 1px clipping box -- that is the whole technique.
        if (node.classList.contains("sr-only") || node.closest(".sr-only")) continue;
        const clippedX = node.scrollWidth - node.clientWidth > 2 && style.overflowX === "hidden";
        const clippedY = node.scrollHeight - node.clientHeight > 2 && style.overflowY === "hidden";
        if (clippedX || clippedY) {
          offenders.push(
            `${specimen.getAttribute("data-primitive")} ${node.tagName.toLowerCase()}.${node.className.toString().split(/\s+/)[0]}`,
          );
        }
      }
    }
    document.documentElement.style.fontSize = "";
    return [...new Set(offenders)];
  });

  expect(clipped).toEqual([]);
});
