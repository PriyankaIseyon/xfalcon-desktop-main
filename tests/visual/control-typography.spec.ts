import { expect, test } from "@playwright/test";
import { installDatasetReportsMock, ok } from "./datasets-reports-native-mock";
import type { ReportView } from "../../src/lib/generated/contracts";

for (const theme of ["light", "dark"] as const) {
  test(`control typography and patterned report cards in ${theme}`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    const reports = [1, 2, 3].map((index) => ({
      version: 1,
      id: `report-${index}`,
      title: `Revenue dashboard ${index}`,
      description: "Revenue, retention, and pipeline summary.",
      created_at_ms: 0,
      updated_at_ms: 0,
      pinned: false,
      hidden: false,
      kind: "dashboard",
      byte_size: 100,
      scope: "user",
      is_owner: true,
      owner_display_name: null,
      refreshable: true,
      can_refresh: true,
      can_manage: true,
      can_delete: true,
      page_count: 1,
      dataset_count: 1,
      source_conversation_id: null,
    })) as unknown as ReportView[];
    await installDatasetReportsMock(page, {
      theme,
      modelCatalog: {
        version: 1,
        selected_model_id: "model-1",
        selection_locked: false,
        models: [
          {
            version: 1,
            id: "model-1",
            provider: "openai",
            display_name: "Test model",
            capabilities: { tools: true, image_input: false, reasoning_summary: false },
          },
        ],
      },
      datasets: [
        ok({
          version: 1,
          datasets: [
            {
              version: 1,
              id: "dataset-1",
              display_name: "Sales",
              status: "ready",
              created_at_ms: 0 as unknown as bigint,
              updated_at_ms: 0 as unknown as bigint,
            },
          ],
          uploads_enabled: true,
          truncated: false,
        }),
      ],
      datasetDetails: [
        ok({
          version: 1,
          uploads_enabled: true,
          dataset: {
            version: 1,
            id: "dataset-1",
            display_name: "Sales",
            status: "ready",
            created_at_ms: 0 as unknown as bigint,
            updated_at_ms: 0 as unknown as bigint,
          },
          files: [
            {
              file_id: "file-1",
              table_name: "sales",
              original_filename: "sales.csv",
              row_count: 10309,
              column_count: 47,
              size_bytes: 1500000,
              created_at_ms: 0 as unknown as bigint,
              updated_at_ms: 0 as unknown as bigint,
              columns: Array.from({ length: 47 }, (_, i) => ({
                name: `sales_column_${i + 1}`,
                inferred_type: ["string", "int64", "date32[day]", "bool"][i % 4],
              })),
            },
          ],
        }),
      ],
      reports: [ok({ version: 1, reports, project_origins: [] })],
      chatSummary: {
        version: 1,
        conversation_id: "chat-1",
        title: "Quarterly review",
        updated_at: new Date().toISOString(),
      },
    });
    await page.goto("/");
    await page.getByRole("button", { name: "Reports", exact: true }).click();
    const cards = page.locator(".report-card");
    await expect(cards).toHaveCount(3);
    await page.evaluate(() => document.documentElement.setAttribute("data-font-size", "medium"));
    await expect(page.getByPlaceholder("Search by name or description")).toHaveCSS("font-size", "14px");
    await expect(cards.first().locator("h3 button")).toHaveCSS("font-size", "16px");
    await expect(cards.first().locator("h3 button")).toHaveCSS("font-weight", "700");
    const patterns = await cards
      .locator(".report-card__grid > svg")
      .evaluateAll((nodes) => nodes.map((node) => node.innerHTML));
    expect(new Set(patterns).size).toBe(3);
    expect(await cards.first().evaluate((node) => getComputedStyle(node).backgroundImage)).toContain("linear-gradient");
    await page.screenshot({ path: `/tmp/xfalcon-reports-${theme}.png` });
    await page.getByRole("button", { name: /^Delete Revenue dashboard 1/ }).click();
    await expect(page.getByRole("alertdialog")).toBeVisible();
    await page.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(cards).toHaveCount(3);
    await page.getByRole("button", { name: "Datasets", exact: true }).click();
    await expect(page.getByPlaceholder("Search by name", { exact: true })).toHaveCSS("font-size", "14px");
    const upload = page.getByRole("button", { name: "Upload dataset", exact: true }).first();
    await expect(upload).toHaveCSS("font-size", "12px");
    await expect(upload).toHaveCSS("font-weight", "500");
    await page.getByText("Sales", { exact: true }).click();
    const schema = page.getByRole("dialog", { name: "Sales", exact: true });
    await expect(schema.getByRole("listitem")).toHaveCount(47);
    const overflow = await schema.locator(".schema-columns__list").evaluate((node) => ({
      vertical: node.scrollHeight > node.clientHeight,
      horizontal: node.scrollWidth > node.clientWidth,
    }));
    expect(overflow).toEqual({ vertical: true, horizontal: false });
    await page.screenshot({ path: `/tmp/xfalcon-schema-${theme}.png` });
    const frame = await schema.boundingBox();
    const searchFrame = await schema.getByRole("searchbox").boundingBox();
    await schema.getByRole("searchbox").fill("date32");
    await expect(schema.getByRole("listitem")).toHaveCount(12);
    for (const query of ["sales_column_47", "no matching column", ""]) {
      await schema.getByRole("searchbox").fill(query);
      await expect(async () => {
        expect(await schema.boundingBox()).toEqual(frame);
        expect(await schema.getByRole("searchbox").boundingBox()).toEqual(searchFrame);
      }).toPass({ timeout: 5000 });
    }
    const originalViewport = page.viewportSize()!;
    await page.setViewportSize({ width: 720, height: 600 });
    await page.evaluate(() => document.documentElement.setAttribute("data-font-size", "large"));
    await expect(async () => {
      const bounds = (await schema.boundingBox())!;
      expect(bounds.x).toBeGreaterThanOrEqual(0);
      expect(bounds.y).toBeGreaterThanOrEqual(0);
      expect(bounds.x + bounds.width).toBeLessThanOrEqual(720);
      expect(bounds.y + bounds.height).toBeLessThanOrEqual(600);
    }).toPass({ timeout: 5000 });
    const smallFrame = await schema.boundingBox();
    await schema.getByRole("searchbox").fill("no matching column");
    expect(await schema.boundingBox()).toEqual(smallFrame);
    await schema.getByRole("button", { name: "Close", exact: true }).click();
    await page.setViewportSize(originalViewport);
    await page.evaluate(() => document.documentElement.setAttribute("data-font-size", "medium"));

    await page.getByRole("button", { name: "Chats", exact: true }).click();
    await page.getByRole("button", { name: "Delete Quarterly review" }).click();
    const dialog = page.getByRole("alertdialog", { name: "Delete chat?" });
    for (const name of ["Cancel", "Delete chat"]) {
      await expect(dialog.getByRole("button", { name, exact: true })).toHaveCSS("font-size", "12px");
      await expect(dialog.getByRole("button", { name, exact: true })).toHaveCSS("font-weight", "500");
    }
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await page.keyboard.press("Control+k");
    const palette = page.getByRole("dialog", { name: "Search" });
    await expect(palette).toBeVisible();
    await expect(palette).toHaveCSS("outline-width", "1px");
    await expect(palette).toHaveCSS("outline-offset", "2px");
    const paletteFrame = await palette.boundingBox();
    for (const query of ["Reports", "zzzz-no-match", ""]) {
      await palette.getByRole("combobox").fill(query);
      await expect(async () => expect(await palette.boundingBox()).toEqual(paletteFrame)).toPass({ timeout: 5000 });
    }
    await page.keyboard.press("Escape");
    const composer = page.getByRole("textbox", { name: "Message xFalcon", exact: true });
    await composer.fill("/");
    const suggestions = page.getByRole("listbox", { name: "Slash commands" });
    await expect(suggestions).toBeVisible();
    await expect(suggestions).toHaveCSS("outline-offset", "2px");
    const suggestionsFrame = await suggestions.boundingBox();
    await composer.fill("/zzzz-no-match");
    await expect(suggestions.getByRole("status")).toContainText("No matching");
    expect(await suggestions.boundingBox()).toEqual(suggestionsFrame);
    await composer.fill("");
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    const model = page.getByRole("button", { name: "Default model", exact: true });
    await expect(model).toHaveCSS("font-size", "14px");
    await model.click();
    await expect(page.getByRole("menu")).toHaveCSS("outline-width", "1px");
    await expect(page.getByRole("menu")).toHaveCSS("outline-offset", "2px");
    await expect(page.getByRole("menuitemradio", { name: "Test model" })).toHaveCSS("font-size", "14px");
  });
}

for (const width of [1440, 1024, 720]) {
  test(`compact report rows stay left-aligned at ${width}px`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.setViewportSize({ width: 1440, height: 960 });
    const reports = Array.from({ length: 7 }, (_, index) => ({
      version: 1,
      id: `layout-${index}`,
      title: `Quarterly report ${index}`,
      description: "Revenue and customer retention across all regions.",
      created_at_ms: Date.now(),
      updated_at_ms: Date.now(),
      pinned: index < 4,
      hidden: false,
      kind: "dashboard",
      byte_size: 100,
      scope: "user",
      is_owner: true,
      owner_display_name: null,
      refreshable: true,
      can_refresh: true,
      can_manage: true,
      can_delete: true,
      page_count: 1,
      dataset_count: 1,
      source_conversation_id: null,
    })) as unknown as ReportView[];
    await installDatasetReportsMock(page, { reports: [ok({ version: 1, reports, project_origins: [] })] });
    await page.goto("/");
    await page.getByRole("button", { name: "Reports", exact: true }).click();
    await expect(page.locator(".report-card")).toHaveCount(7);
    await page.setViewportSize({ width, height: 960 });
    for (const name of ["Pinned", "Recent"]) {
      const section = page.getByRole("region", { name, exact: true });
      await expect(async () => {
        const box = await section.boundingBox();
        expect(box).not.toBeNull();
        const cards = await section.locator(".report-card").evaluateAll((nodes) =>
          nodes.map((node) => {
            const { x, y, width, height } = node.getBoundingClientRect();
            return { x, y, width, height, overflow: node.scrollWidth > node.clientWidth };
          }),
        );
        const rows = new Map<number, typeof cards>();
        for (const card of cards) {
          rows.set(card.y, [...(rows.get(card.y) ?? []), card]);
          expect(card.overflow).toBe(false);
          expect(card.height).toBeGreaterThanOrEqual(168);
          if (width === 1440) expect(card.width).toBeCloseTo(((box!.width - 24) / 3) * 0.6, 0);
        }
        for (const row of rows.values()) {
          expect(row.length).toBeLessThanOrEqual(3);
          expect(row[0].x).toBeCloseTo(box!.x, 0);
        }
      }).toPass({ timeout: 5000 });
    }
    await page.screenshot({ path: `/tmp/xfalcon-compact-reports-${width}.png` });
  });
}
