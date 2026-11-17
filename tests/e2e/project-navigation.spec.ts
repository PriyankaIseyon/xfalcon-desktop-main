import { expect, test } from "@playwright/test";
import { installChatMocks } from "./chat-mocks";

const projectName = "Research project with a long name for quarterly enterprise revenue analysis";
const projectChat = "Write complete project files and review the results across the entire workspace";
const chatNames = [
  "Compare customer retention across international enterprise accounts for this quarter",
  "Review revenue trends and monthly operating costs across every regional department",
];

for (const theme of ["light", "dark"] as const) {
  test(`project navigation contains labels and keeps connectors clear in ${theme}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width: 1440, height: 960 });
    await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
    await installChatMocks(page, {
      appearanceTheme: theme,
      projects: [
        {
          version: 1,
          id: "project-navigation",
          display_name: projectName,
          root_path: "C:\\work\\research",
          status: "trusted",
          active: true,
          trusted_at_ms: 0,
        },
      ],
      projectDirectories: [
        {
          version: 1,
          project_id: "project-navigation",
          path: null,
          entries: [{ name: "src", kind: "directory", size_bytes: 0, is_sensitive: false }],
        },
        {
          version: 1,
          project_id: "project-navigation",
          path: "src",
          entries: [
            { name: "main.ts", kind: "file", size_bytes: 19, is_sensitive: false },
            { name: "nested", kind: "directory", size_bytes: 0, is_sensitive: false },
          ],
        },
        {
          version: 1,
          project_id: "project-navigation",
          path: "src/nested",
          entries: [{ name: "deep.ts", kind: "file", size_bytes: 19, is_sensitive: false }],
        },
      ],
      bootstrapConversations: [
        { conversation_id: "loose-1", title: chatNames[0], updated_at: new Date().toISOString() },
        { conversation_id: "loose-2", title: chatNames[1], updated_at: "2026-01-01T12:00:00Z" },
        { conversation_id: "project-chat", title: projectChat, project_id: "project-navigation" },
      ],
    });
    await page.goto("/");
    await expect(page.getByRole("option", { name: chatNames[0], exact: true })).toBeVisible();
    await page.mouse.move(1430, 950);
    for (const name of chatNames) {
      const row = page.getByRole("option", { name, exact: true });
      const label = row.locator(".sidebar-row__title");
      const rowBox = (await row.boundingBox())!;
      const labelBox = (await label.boundingBox())!;
      expect(rowBox.x + rowBox.width - labelBox.x - labelBox.width).toBeLessThanOrEqual(9);
      await label.hover();
      await expect(label).toHaveAttribute("title", name);
      const actions = row.locator("..").getByRole("button", { name: `Pin ${name}`, exact: true });
      const deletion = row.locator("..").getByRole("button", { name: `Delete ${name}`, exact: true });
      await expect(actions.locator("..")).toHaveCSS("opacity", "1");
      await expect(deletion).toBeVisible();
      const actionBox = (await actions.boundingBox())!;
      const hoveredLabel = (await label.boundingBox())!;
      expect(hoveredLabel.x + hoveredLabel.width).toBeLessThanOrEqual(actionBox.x);
      await page.mouse.move(1430, 950);
    }

    await page.getByRole("button", { name: "Projects", exact: true }).click();
    const tree = page.getByRole("tree", { name: "Project files" });
    await expect(tree).toBeVisible();
    for (const name of ["nested", "main.ts", "deep.ts"]) {
      const row = tree.getByRole("treeitem", { name: new RegExp(name.replace(".", "\\.")) });
      await expect(row).toBeVisible();
      await row.hover();
      const geometry = await row.evaluate((element) => ({
        highlightLeft: element.getBoundingClientRect().left + parseFloat(getComputedStyle(element, "::before").left),
        guideRight: Math.max(
          ...[...element.querySelectorAll(".project-tree__guide")].map((guide) => guide.getBoundingClientRect().right),
        ),
      }));
      expect(geometry.highlightLeft).toBeGreaterThan(geometry.guideRight);
    }
    await tree.getByRole("treeitem", { name: /main\.ts/ }).click();
    await expect(tree.getByRole("treeitem", { name: /main\.ts/ })).toHaveAttribute("aria-selected", "true");
    await page.screenshot({ path: testInfo.outputPath("project-files.png") });

    const projectRow = page.locator(".sidebar-tree .sidebar-row").filter({ hasText: projectChat }).first();
    await expect(projectRow).toBeVisible();
    await projectRow.click();
    await expect(page.getByRole("main", { name: "Project chat" })).toBeVisible();
    const header = page.locator(".app-titlebar");
    for (const readingSize of ["small", "medium", "large"]) {
      await page.evaluate((value) => {
        document.documentElement.dataset.fontSize = value;
      }, readingSize);
      const box = (await header.boundingBox())!;
      const subtitle = header.getByText(projectName, { exact: true });
      const subtitleBox = (await subtitle.boundingBox())!;
      expect(subtitleBox.y + subtitleBox.height).toBeLessThanOrEqual(box.y + box.height - 1);
      await subtitle.hover();
      if (await subtitle.evaluate((element) => element.scrollWidth > element.clientWidth)) {
        await expect(subtitle).toHaveAttribute("title", projectName);
      }
      await header.getByRole("button", { name: "Rename conversation" }).click();
      for (const control of await header.getByRole("button").all()) {
        const controlBox = (await control.boundingBox())!;
        expect(controlBox.y).toBeGreaterThanOrEqual(box.y);
        expect(controlBox.y + controlBox.height).toBeLessThanOrEqual(box.y + box.height);
      }
      const editingSubtitle = (await subtitle.boundingBox())!;
      expect(editingSubtitle.y + editingSubtitle.height).toBeLessThanOrEqual(box.y + box.height - 1);
      await header.getByRole("button", { name: "Cancel rename" }).click();
    }
    await page.getByRole("button", { name: `New chat in ${projectName}`, exact: true }).click();
    await expect(header.getByRole("button", { name: "Rename conversation" })).toHaveCount(0);
    for (const readingSize of ["small", "medium", "large"]) {
      await page.evaluate((value) => {
        document.documentElement.dataset.fontSize = value;
      }, readingSize);
      const box = (await header.boundingBox())!;
      const subtitle = (await header.getByText(projectName, { exact: true }).boundingBox())!;
      expect(subtitle.y + subtitle.height).toBeLessThanOrEqual(box.y + box.height - 1);
    }
    await page.screenshot({ path: testInfo.outputPath("project-chat.png") });
  });
}
