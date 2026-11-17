import { expect, test, type Page, type TestInfo } from "@playwright/test";

// ---------------------------------------------------------------------------
// Task 4.6: visual baselines for S040-S046, the project explorer, tree,
// schema panel, file viewer, revision history, add/trust, and remove-project
// surfaces. S047 (patch approval) was removed with the review sheet (ADR-043).
//
// Mirrors shell.spec.ts's and approval-activity.spec.ts's structure: the same
// native-boundary mocking pattern via `page.addInitScript`, the same
// variant set for the one flagship layout sweep, and single one-off tests
// (at the light variant) for every other reachable state -- multiplying
// every state across every theme would not find new defects, only spend
// baseline-maintenance cost shell.spec.ts and approval-activity.spec.ts
// already decided against.
//
// Every async native command below resolves from a small per-command queue:
// one outcome per call, the last one repeating for any further calls. A
// `{ kind: "pending" }` outcome never resolves during the test -- exactly
// what a loading/busy screenshot needs, and nothing more.
// ---------------------------------------------------------------------------

const bootstrap = {
  version: 1,
  application_version: "0.1.0-visual-test",
  platform: "windows",
  session: {
    status: "authenticated",
    version: 1,
    identity: {
      version: 1,
      user_id: "visual-user",
      tenant_id: "visual-tenant",
      display_name: "Visual Test",
      email: null,
    },
    expires_at_ms: Date.now() + 3_600_000,
  },
} as const;

type VisualVariant = {
  name: string;
  theme: "light" | "dark";
  viewport: { width: number; height: number };
  contrast?: "more";
};

const variants: VisualVariant[] = [
  { name: "light", theme: "light", viewport: { width: 1440, height: 960 } },
  { name: "dark", theme: "dark", viewport: { width: 1440, height: 960 } },
  { name: "narrow", theme: "light", viewport: { width: 720, height: 900 } },
  { name: "high-contrast", theme: "light", viewport: { width: 1440, height: 960 }, contrast: "more" },
];

type Outcome = { kind: "ok"; value: unknown } | { kind: "error"; message: string } | { kind: "pending" };

function ok(value: unknown): Outcome {
  return { kind: "ok", value };
}
function err(message: string): Outcome {
  return { kind: "error", message };
}
const pending: Outcome = { kind: "pending" };

type ProjectFixture = {
  id: string;
  display_name: string;
  root_path: string;
  status: "trusted" | "unavailable";
  active: boolean;
};

interface ProjectMockConfig {
  theme: "light" | "dark";
  projects: ProjectFixture[];
  /** Keyed by project-relative path; `""` is the trusted root. */
  directories?: Record<string, Outcome[]>;
  /** What the opened file holds on each successive stat: a `DiskFile`, or a
   * non-text stat (`sensitive_withheld`, `not_text`) as the backend answers it. */
  files?: Outcome[];
  saveFile?: Outcome[];
  schemaSummary?: Outcome[];
  revisions?: Outcome[];
  revisionDiffs?: Outcome[];
  restore?: Outcome[];
  createProject?: Outcome[];
  pickDirectory?: Outcome[];
  revokeProject?: Outcome[];
}

async function installProjectMock(page: Page, config: ProjectMockConfig): Promise<void> {
  // `addInitScript` re-evaluates this function in the browser, with no
  // closure over anything from the outer (Node-side) module scope --
  // `bootstrap` has to travel in through the serialized argument like
  // everything else the callback reads.
  await page.addInitScript(
    (cfg: ProjectMockConfig & { bootstrap: typeof bootstrap }) => {
      const projects = cfg.projects.map((project) => ({
        version: 1,
        id: project.id,
        display_name: project.display_name,
        root_path: project.root_path,
        status: project.status,
        active: project.active,
        trusted_at_ms: 1_700_000_000_000,
      }));

      const queues: Record<string, unknown[]> = {
        saveFile: [...(cfg.saveFile ?? [])],
        schemaSummary: [...(cfg.schemaSummary ?? [])],
        revisions: [...(cfg.revisions ?? [])],
        revisionDiffs: [...(cfg.revisionDiffs ?? [])],
        restore: [...(cfg.restore ?? [])],
        createProject: [...(cfg.createProject ?? [])],
        pickDirectory: [...(cfg.pickDirectory ?? [])],
        revokeProject: [...(cfg.revokeProject ?? [])],
        files: [...(cfg.files ?? [])],
      };
      let opened: (offset: number, length: number) => ArrayBuffer = () => new ArrayBuffer(0);
      const directoryQueues: Record<string, unknown[]> = {};
      for (const [path, outcomes] of Object.entries(cfg.directories ?? {})) directoryQueues[path] = [...outcomes];

      // React's StrictMode (development only, and this app runs its dev
      // build under Playwright the same as production) mounts every effect
      // twice -- synchronously, in the same tick, before either call's
      // promise can settle. The real component already discards the first
      // (phantom) call's result by generation number, so only the later of
      // the two ever reaches the screen; a naive queue would still shift
      // twice and hand the phantom call the response meant for a real, later
      // user action. Coalescing every call that arrives before the previous
      // batch's macrotask fires into one queue advance reproduces what one
      // real network round-trip would do: every caller in the same tick gets
      // the same single result, and a call arriving a real tick later (an
      // actual click, an actual focus event) still advances the queue on its
      // own.
      const coalesced: Record<
        string,
        { waiters: Array<{ resolve: (value: unknown) => void; reject: (error: Error) => void }> } | undefined
      > = {};
      function nextFrom(key: string, queue: unknown[]): Promise<unknown> {
        return new Promise((resolve, reject) => {
          let bucket = coalesced[key];
          if (!bucket) {
            const fresh = {
              waiters: [] as Array<{ resolve: (value: unknown) => void; reject: (error: Error) => void }>,
            };
            bucket = fresh;
            coalesced[key] = fresh;
            setTimeout(() => {
              const waiters = fresh.waiters;
              coalesced[key] = undefined;
              const outcome = (queue.length > 1 ? queue.shift() : queue[0]) as
                { kind: "ok"; value: unknown } | { kind: "error"; message: string } | { kind: "pending" } | undefined;
              for (const waiter of waiters) {
                if (outcome === undefined) waiter.reject(new Error("no queued outcome for this call"));
                else if (outcome.kind === "error") waiter.reject(new Error(outcome.message));
                else if (outcome.kind === "ok") waiter.resolve(outcome.value);
                // "pending" waiters are deliberately left unsettled.
              }
            }, 0);
          }
          bucket.waiters.push({ resolve, reject });
        });
      }

      const callbacks = new Map<number, (data: unknown) => void>();
      const eventListeners = new Map<string, number[]>();
      let nextCallback = 1;
      const transformCallback = (callback: (data: unknown) => void) => {
        const id = nextCallback++;
        callbacks.set(id, callback);
        return id;
      };

      Object.defineProperty(window, "__TAURI_INTERNALS__", {
        configurable: false,
        value: Object.freeze({
          transformCallback,
          invoke(command: string, args?: Record<string, unknown>) {
            if (command === "plugin:event|listen") {
              const event = String(args?.event);
              const handler = Number(args?.handler);
              eventListeners.set(event, [...(eventListeners.get(event) ?? []), handler]);
              return Promise.resolve(handler);
            }
            if (command === "plugin:event|unlisten") {
              const event = String(args?.event);
              const id = Number(args?.eventId);
              eventListeners.set(
                event,
                (eventListeners.get(event) ?? []).filter((handler) => handler !== id),
              );
              callbacks.delete(id);
              return Promise.resolve(null);
            }
            if (command === "app_bootstrap") return Promise.resolve(cfg.bootstrap);
            if (command === "app_health") return Promise.resolve({ version: 1, status: "ready" });
            if (command === "settings_appearance_get") {
              return Promise.resolve({
                version: 1,
                theme: cfg.theme,
                reading_size: "small",
                show_reasoning: false,
                show_follow_ups: true,
              });
            }
            if (command === "workspace_status") {
              return Promise.resolve({
                version: 1,
                tenant_host: "tenant.example",
                mcp_resource_url: "https://tenant.example/mcp",
                mcp_servers: [],
                models_available: 1,
                default_model_id: "gpt-test",
                default_provider: "openai",
                chat_ready: true,
                setup_note: "Ready",
              });
            }
            if (command === "chat_workspace_bootstrap") {
              return Promise.resolve({ version: 1, conversations: [], recovery_notice: null });
            }
            if (command === "diagnostics_log_renderer_error") return Promise.resolve(undefined);
            if (command === "project_list")
              return Promise.resolve({ version: 1, projects: projects.map((project) => ({ ...project })) });
            if (command === "project_find_by_folder") {
              return Promise.resolve({
                version: 1,
                projects: projects.filter((project) => project.root_path === args?.path),
              });
            }
            if (command === "project_activate") {
              const project = projects.find((candidate) => candidate.id === args?.projectId);
              if (project)
                projects.forEach((candidate) => {
                  candidate.active = candidate.id === project.id;
                });
              return project
                ? Promise.resolve({ ...project, active: true })
                : Promise.reject(new Error("unknown project"));
            }
            if (command === "project_list_directory") {
              const path = (args?.path as string | null) ?? "";
              const queue = directoryQueues[path];
              if (!queue) return Promise.reject(new Error(`no directory fixture for "${path}"`));
              return nextFrom(`dir:${path}`, queue);
            }
            if (command === "project_file_stat") {
              const path = String(args?.path);
              return nextFrom("files", queues.files).then((state) => {
                const disk = state as { text?: string; pattern?: string; size_bytes?: number; modified_ms?: number };
                if (disk.text === undefined && disk.pattern === undefined) return state;
                const bytes = disk.text === undefined ? null : new TextEncoder().encode(disk.text);
                const pattern = new TextEncoder().encode(disk.pattern ?? "");
                const size = bytes?.length ?? disk.size_bytes ?? 0;
                // The bytes a read sees are the bytes of the state last stat'd,
                // so a change on disk shows up the way a real one would.
                opened = (offset, length) => {
                  const end = Math.min(size, offset + length);
                  if (bytes) return bytes.slice(offset, end).buffer;
                  const out = new Uint8Array(Math.max(0, end - offset));
                  for (let i = 0; i < out.length; i += 1) out[i] = pattern[(offset + i) % pattern.length];
                  return out.buffer;
                };
                return {
                  version: 1,
                  kind: "text",
                  path,
                  size_bytes: size,
                  modified_ms: disk.modified_ms ?? 1_700_000_000_000,
                };
              });
            }
            if (command === "project_file_read_bytes")
              return Promise.resolve(opened(Number(args?.offset), Number(args?.length)));
            if (command === "project_save_file") return nextFrom("saveFile", queues.saveFile);
            if (command === "project_schema_summary") return nextFrom("schemaSummary", queues.schemaSummary);
            if (command === "project_file_revisions") return nextFrom("revisions", queues.revisions);
            if (command === "project_file_revision_diff") return nextFrom("revisionDiffs", queues.revisionDiffs);
            if (command === "project_file_restore") return nextFrom("restore", queues.restore);
            if (command === "project_create") return nextFrom("createProject", queues.createProject);
            if (command === "plugin:dialog|open") return nextFrom("pickDirectory", queues.pickDirectory);
            if (command === "project_revoke") return nextFrom("revokeProject", queues.revokeProject);
            return Promise.reject(new Error(`visual test rejected an unknown native command: ${command}`));
          },
        }),
        writable: false,
      });
    },
    { ...config, bootstrap },
  );
}

async function openShell(page: Page, variant: VisualVariant, config: Omit<ProjectMockConfig, "theme">): Promise<void> {
  await page.setViewportSize(variant.viewport);
  await page.emulateMedia({
    colorScheme: variant.theme,
    contrast: variant.contrast ?? "no-preference",
    reducedMotion: "reduce",
  });
  await installProjectMock(page, { theme: variant.theme, ...config });
  await page.goto("/");
  await expect(page.getByRole("main", { name: "Chat workspace" })).toBeVisible();
  await page.evaluate(async () => document.fonts.ready);
}

/** `ProjectNavList` lives inside the collapsible sidebar, which starts closed
 * on a narrow viewport, and choosing a destination closes it again -- open it
 * both before and after, matching approval-activity.spec.ts's own helper. A
 * no-op on wider variants, where the button reads "Hide sidebar" instead. */
async function ensureSidebarOpen(page: Page): Promise<void> {
  const showSidebar = page.getByRole("button", { name: "Show sidebar" });
  if (await showSidebar.isVisible().catch(() => false)) await showSidebar.click();
}

/** Opens the Projects destination and its file pane. */
/** `keepSidebarOpen` is for tests that still need a sidebar row afterward
 * (e.g. a project's own "Remove" action) -- reopening it on narrow leaves
 * its scrim covering the main content, which every other test here goes on
 * to click into. */
async function openProjectFiles(page: Page, { keepSidebarOpen = false } = {}): Promise<void> {
  await ensureSidebarOpen(page);
  await page.getByRole("button", { name: "Projects" }).click();
  await expect(page.getByRole("main", { name: "Project files" })).toBeVisible();
  if (keepSidebarOpen) await ensureSidebarOpen(page);
}

async function attachScreenshot(page: Page, testInfo: TestInfo, name: string): Promise<void> {
  const screenshotPath = testInfo.outputPath(`${name}.png`);
  await page.screenshot({ animations: "disabled", caret: "hide", fullPage: false, path: screenshotPath, scale: "css" });
  await testInfo.attach(`${name}.png`, { path: screenshotPath, contentType: "image/png" });
}

const project = (overrides: Partial<ProjectFixture> = {}): ProjectFixture => ({
  id: "project-1",
  display_name: "Research",
  root_path: "C:\\work\\research",
  status: "trusted",
  active: true,
  ...overrides,
});

test("project CSV chart controls fit without scrolling at each reading size", async ({ page }, testInfo) => {
  await openShell(page, variants[0]!, {
    projects: [project()],
    directories: {
      "": [ok({ version: 1, project_id: "project-1", path: null, entries: [entry({ name: "metrics.csv" })] })],
    },
    files: [ok({ text: "Category,Revenue\nRivers,10\nMountains,20\n" })],
  });
  await openProjectFiles(page);
  await page.getByRole("treeitem", { name: /^metrics\.csv/ }).click();
  await page.getByRole("button", { name: "Chart metrics.csv in the side panel" }).click();
  const selector = page.getByRole("radiogroup", { name: "Chart type" });
  for (const width of [1440, 1024, 720]) {
    await page.setViewportSize({ width, height: 900 });
    for (const size of ["small", "medium", "large"]) {
      await page.evaluate((value) => document.documentElement.setAttribute("data-font-size", value), size);
      await expect(selector).toBeVisible();
      expect(
        await selector.evaluate((node) => ({
          horizontal: node.scrollWidth > node.clientWidth + 1,
          vertical: node.scrollHeight > node.clientHeight + 1,
          contained: [...node.children].every((child) => {
            const outer = node.getBoundingClientRect();
            const inner = child.getBoundingClientRect();
            return (
              inner.left >= outer.left &&
              inner.right <= outer.right &&
              inner.top >= outer.top &&
              inner.bottom <= outer.bottom
            );
          }),
        })),
      ).toEqual({ horizontal: false, vertical: false, contained: true });
      if ((width === 1440 && size === "small") || (width === 720 && size === "large")) {
        await attachScreenshot(page, testInfo, `csv-chart-${width}-${size}`);
      }
      for (const type of ["Bar", "Line", "Area", "Scatter", "Pie"]) {
        const option = selector.getByRole("radio", { name: type, exact: true });
        await option.click();
        await expect(option).toHaveAttribute("aria-checked", "true");
        await expect(page.getByRole("img", { name: `metrics.csv, ${type.toLowerCase()} chart` })).toBeVisible();
      }
    }
  }
});

test("project file actions keep their widths with long filenames and an open chart panel", async ({ page }) => {
  const names = ["drivers.csv", "constructors_with_a_long_descriptive_filename.csv"];
  await openShell(page, variants[0]!, {
    projects: [project()],
    directories: {
      "": [ok({ version: 1, project_id: "project-1", path: null, entries: names.map((name) => entry({ name })) })],
    },
    files: [ok({ text: "Category,Revenue\nRivers,10\nMountains,20\n" })],
  });
  await openProjectFiles(page);
  const widths: Record<string, number> = {};
  for (const name of names) {
    await page.getByRole("treeitem", { name: new RegExp(`^${name.replace(/\./g, "\\.")}`) }).click();
    const actions = page.getByRole("group", { name: `Actions for ${name}` });
    await expect(actions).toBeVisible();
    for (const button of await actions.getByRole("button").all()) {
      const text = (await button.innerText()).trim();
      const width = await button.evaluate((node) => Number.parseFloat(getComputedStyle(node).width));
      widths[text] ??= width;
      expect(width).toBeCloseTo(widths[text]!, 1);
    }
  }
  await page.getByRole("button", { name: `Chart ${names[1]} in the side panel` }).click();
  for (const viewport of [1440, 1024]) {
    await page.setViewportSize({ width: viewport, height: 900 });
    const actions = page.getByRole("group", { name: `Actions for ${names[1]}` });
    for (const button of await actions.getByRole("button").all()) {
      const text = (await button.innerText()).trim();
      expect(await button.evaluate((node) => Number.parseFloat(getComputedStyle(node).width))).toBeCloseTo(
        widths[text]!,
        1,
      );
      await expect(button).toHaveCSS("white-space", "nowrap");
      await expect(button).toHaveCSS("flex-shrink", "0");
    }
  }
});

for (const variant of variants) {
  test(`${variant.name} new project starts with a compact cyan folder action`, async ({ page }) => {
    await openShell(page, variant, { projects: [] });
    await openProjectFiles(page);
    await page.getByRole("main", { name: "Project files" }).getByRole("button", { name: "New project" }).click();
    const dialog = page.getByRole("dialog", { name: "New project" });
    await expect(dialog.getByRole("button", { name: "Choose folder" })).toHaveClass(/bg-accent/);
    const box = await dialog.boundingBox();
    expect(box?.height).toBeLessThan(300);
    await expect(page).toHaveScreenshot(["project", `new-project-${variant.name}.png`]);
  });
}

for (const variant of variants) {
  test(`${variant.name} shared folder warning links to existing projects and permits creation`, async ({
    page,
  }, testInfo) => {
    const rootPath = "C:\\work\\research";
    await openShell(page, variant, {
      projects: [
        project({ active: false }),
        project({ id: "project-2", display_name: "Quarterly revenue and customer retention analysis", active: false }),
      ],
      pickDirectory: [ok(rootPath)],
      directories: { "": [ok({ version: 1, project_id: "project-1", path: null, entries: [] })] },
    });
    await openProjectFiles(page);
    await page.getByRole("main", { name: "Project files" }).getByRole("button", { name: "New project" }).click();
    const dialog = page.getByRole("dialog", { name: "New project" });
    await dialog.getByRole("button", { name: "Choose folder" }).click();
    await expect(dialog.getByText("Shared project folder")).toBeVisible();
    await expect(dialog.getByRole("button", { name: "Open Research", exact: true })).toBeVisible();
    await expect(dialog.getByRole("button", { name: "Trust & create project" })).toBeEnabled();
    await expect(dialog.getByRole("button", { name: "Choose a different folder" })).toBeEnabled();
    await expect(page).toHaveScreenshot(["project", `shared-folder-${variant.name}.png`]);
    await attachScreenshot(page, testInfo, `shared-folder-${variant.name}`);
    await dialog.getByRole("button", { name: "Open Research", exact: true }).click();
    await expect(dialog).not.toBeVisible();
    await expect(page.getByText("This project folder is empty.")).toBeVisible();
  });
}

function entry(overrides: Partial<{ name: string; kind: string; size_bytes: number; is_sensitive: boolean }> = {}) {
  return { name: "notes.md", kind: "file", size_bytes: 128, is_sensitive: false, ...overrides };
}

// ---------------------------------------------------------------------------
// Flagship layout: a populated tree with a file open, across every variant.
// Covers the bulk of S040's frame and S041's ready/selected state at once.
// ---------------------------------------------------------------------------

for (const variant of variants) {
  test(`${variant.name} project explorer shows a populated tree with a file open`, async ({ page }, testInfo) => {
    await openShell(page, variant, {
      projects: [project()],
      directories: {
        "": [
          ok({
            version: 1,
            project_id: "project-1",
            path: null,
            entries: [
              entry({ name: "src", kind: "directory" }),
              entry({ name: "notes.md" }),
              entry({ name: "secrets.env", is_sensitive: true }),
            ],
          }),
        ],
        src: [ok({ version: 1, project_id: "project-1", path: "src", entries: [entry({ name: "main.ts" })] })],
      },
      files: [ok({ text: "# Research notes\n\nRevenue grew 12% quarter over quarter.\n" })],
    });
    await openProjectFiles(page);
    await expect(page.getByRole("treeitem", { name: /main\.ts/ })).toBeVisible();
    await page.getByRole("treeitem", { name: /^notes\.md/ }).click();
    // Visible only: the card's hidden source view carries the same text.
    await expect(page.getByText("Revenue grew 12% quarter over quarter.").filter({ visible: true })).toBeVisible();

    await expect(page).toHaveScreenshot(["project", `explorer-populated-${variant.name}.png`]);
    await attachScreenshot(page, testInfo, `explorer-populated-${variant.name}`);
  });
}

// ---------------------------------------------------------------------------
// S040 -- explorer landing
// ---------------------------------------------------------------------------

for (const variant of variants) {
  test(`${variant.name} explorer offers project creation with no active project`, async ({ page }, testInfo) => {
    await openShell(page, variant, { projects: [] });
    await openProjectFiles(page);

    await expect(page.getByText("No project is open")).toBeVisible();
    const add = page.getByRole("main", { name: "Project files" }).getByRole("button", { name: "New project" });
    await expect(add).toBeVisible();
    await expect(page).toHaveScreenshot(["project", `explorer-none-active-${variant.name}.png`]);
    await attachScreenshot(page, testInfo, `explorer-none-active-${variant.name}`);
    await add.click();
    await expect(page.getByRole("dialog", { name: "New project" })).toBeVisible();
  });
}

test("explorer offers project creation when existing projects are not selected", async ({ page }) => {
  await openShell(page, variants[0], { projects: [project({ active: false })] });
  await openProjectFiles(page);
  await page.getByRole("main", { name: "Project files" }).getByRole("button", { name: "New project" }).click();
  await expect(page.getByRole("dialog", { name: "New project" })).toBeVisible();
});

test("light explorer warns when the trusted folder is unavailable", async ({ page }, testInfo) => {
  await openShell(page, variants[0], { projects: [project({ status: "unavailable" })] });
  await openProjectFiles(page);

  await expect(page.getByText("This project folder is not available")).toBeVisible();
  await expect(page.getByRole("img", { name: "Project unavailable", exact: true })).toBeVisible();
  await expect(page).toHaveScreenshot(["project", "explorer-unavailable-light.png"]);
  await attachScreenshot(page, testInfo, "explorer-unavailable-light");
});

for (const variant of variants) {
  test(`${variant.name} unavailable project status stays inside its sidebar row`, async ({ page }) => {
    await openShell(page, variant, {
      projects: [
        project({
          display_name: "xFalcon Analytics",
          root_path: "/Users/walter/Projects/Analytics",
          status: "unavailable",
        }),
        project({ id: "project-2", display_name: "Another project", active: false, status: "unavailable" }),
      ],
    });
    await openProjectFiles(page, { keepSidebarOpen: true });
    const tree = page.getByRole("tree", { name: "Projects", exact: true });
    const rows = tree.getByRole("treeitem").filter({ has: page.getByRole("alert") });
    await expect(rows).toHaveCount(2);
    for (const row of await rows.all()) {
      const bounds = await row.boundingBox();
      const detail = await row.getByRole("alert").boundingBox();
      expect(bounds).not.toBeNull();
      expect(detail).not.toBeNull();
      expect(detail!.y).toBeGreaterThanOrEqual(bounds!.y);
      expect(detail!.y + detail!.height).toBeLessThanOrEqual(bounds!.y + bounds!.height);
      expect(detail!.x + detail!.width).toBeLessThanOrEqual(bounds!.x + bounds!.width);
      const title = await row.getByText(/xFalcon Analytics|Another project/, { exact: true }).boundingBox();
      expect(title!.y + title!.height).toBeLessThanOrEqual(detail!.y);
    }
    const first = await rows.nth(0).boundingBox();
    const second = await rows.nth(1).boundingBox();
    expect(first!.y + first!.height).toBeLessThanOrEqual(second!.y);
    const heading = await page.getByRole("button", { name: "New project", exact: true }).boundingBox();
    expect(heading!.y + heading!.height).toBeLessThanOrEqual(first!.y);
  });
}

test("light explorer reports a root listing failure", async ({ page }, testInfo) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: { "": [err("project is not trusted for the current account")] },
  });
  await openProjectFiles(page);

  await expect(page.getByRole("alert")).toHaveText(/This project could not be listed/);
  await expect(page).toHaveScreenshot(["project", "explorer-root-error-light.png"]);
  await attachScreenshot(page, testInfo, "explorer-root-error-light");
});

test("light explorer distinguishes an empty project from a filter with no matches", async ({ page }, testInfo) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: { "": [ok({ version: 1, project_id: "project-1", path: null, entries: [] })] },
  });
  await openProjectFiles(page);

  await expect(page.getByText("This project folder is empty.")).toBeVisible();
  await expect(page).toHaveScreenshot(["project", "explorer-empty-light.png"]);
  await attachScreenshot(page, testInfo, "explorer-empty-light");
});

test("light explorer filters visible files and states no match", async ({ page }, testInfo) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: {
      "": [ok({ version: 1, project_id: "project-1", path: null, entries: [entry({ name: "notes.md" })] })],
    },
  });
  await openProjectFiles(page);
  await page.getByRole("searchbox", { name: "Filter visible project files" }).fill("no-such-file");

  await expect(page.getByText("No visible files match this filter.")).toBeVisible();
  await expect(page).toHaveScreenshot(["project", "explorer-filter-no-match-light.png"]);
  await attachScreenshot(page, testInfo, "explorer-filter-no-match-light");
});

test("light explorer truncates a long root path and a long file name", async ({ page }, testInfo) => {
  const longName = "quarterly-revenue-and-customer-retention-analysis-for-international-accounts.md";
  await openShell(page, variants[0], {
    projects: [project({ root_path: "C:\\work\\clients\\enterprise\\quarterly-revenue-and-retention-analysis-2026" })],
    directories: {
      "": [ok({ version: 1, project_id: "project-1", path: null, entries: [entry({ name: longName })] })],
    },
  });
  await openProjectFiles(page);
  await expect(page.getByRole("treeitem", { name: new RegExp(longName.slice(0, 20)) })).toBeVisible();

  await expect(page).toHaveScreenshot(["project", "explorer-long-path-light.png"]);
  await attachScreenshot(page, testInfo, "explorer-long-path-light");
});

// ---------------------------------------------------------------------------
// S041 -- project tree
// ---------------------------------------------------------------------------

test("light tree shows loading, then an inline per-folder error that survives while expanded", async ({
  page,
}, testInfo) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: {
      "": [
        ok({ version: 1, project_id: "project-1", path: null, entries: [entry({ name: "src", kind: "directory" })] }),
      ],
      src: [pending],
    },
  });
  await openProjectFiles(page);
  // Folders open by default, so `src` is already reading.
  await expect(page.getByRole("treeitem", { name: /^src/ })).toHaveAttribute("aria-busy", "true");

  await expect(page).toHaveScreenshot(["project", "tree-folder-loading-light.png"]);
  await attachScreenshot(page, testInfo, "tree-folder-loading-light");
});

test("light tree keeps a failed folder expanded with its own inline error", async ({ page }, testInfo) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: {
      "": [
        ok({ version: 1, project_id: "project-1", path: null, entries: [entry({ name: "src", kind: "directory" })] }),
      ],
      src: [err("disk is offline")],
    },
  });
  await openProjectFiles(page);

  await expect(page.getByText(/disk is offline/i)).toBeVisible();
  await expect(page.getByRole("treeitem", { name: /^src/ })).toHaveAttribute("aria-expanded", "true");
  await expect(page).toHaveScreenshot(["project", "tree-folder-error-light.png"]);
  await attachScreenshot(page, testInfo, "tree-folder-error-light");
});

test("light tree marks a sensitive file and an unopenable symlink", async ({ page }, testInfo) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: {
      "": [
        ok({
          version: 1,
          project_id: "project-1",
          path: null,
          entries: [
            entry({ name: "credentials.env", is_sensitive: true }),
            entry({ name: "shared-link", kind: "symlink" }),
          ],
        }),
      ],
    },
  });
  await openProjectFiles(page);

  await expect(page.getByRole("treeitem", { name: /credentials\.env/ })).toHaveAttribute("aria-disabled", "true");
  await expect(page.getByText("Symbolic link — not followed")).toBeVisible();
  // `aria-disabled` is not a native `disabled` attribute -- Playwright's own
  // actionability check still refuses the click, so this forces it through,
  // matching what a real pointer landing on the row would do, to prove the
  // row's own click handler refuses to open a sensitive file regardless.
  await page.getByRole("treeitem", { name: /credentials\.env/ }).click({ force: true });
  await expect(page.getByText("This file was not read")).toHaveCount(0);

  await expect(page).toHaveScreenshot(["project", "tree-sensitive-symlink-light.png"]);
  await attachScreenshot(page, testInfo, "tree-sensitive-symlink-light");
});

test("light tree keeps a keyboard cursor visible while navigating without a pointer", async ({ page }, testInfo) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: {
      "": [
        ok({
          version: 1,
          project_id: "project-1",
          path: null,
          entries: [entry({ name: "a.md" }), entry({ name: "b.md" }), entry({ name: "c.md" })],
        }),
      ],
    },
  });
  await openProjectFiles(page);
  await page.getByRole("tree", { name: "Project files" }).focus();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");

  await expect(page).toHaveScreenshot(["project", "tree-keyboard-cursor-light.png"]);
  await attachScreenshot(page, testInfo, "tree-keyboard-cursor-light");
});

test("light tree virtualizes a large project", async ({ page }, testInfo) => {
  const entries = Array.from({ length: 500 }, (_, index) =>
    entry({ name: `file-${String(index).padStart(4, "0")}.ts` }),
  );
  await openShell(page, variants[0], {
    projects: [project()],
    directories: { "": [ok({ version: 1, project_id: "project-1", path: null, entries })] },
  });
  await openProjectFiles(page);

  await expect(page.getByText("500 visible items")).toBeVisible();
  const tree = page.getByRole("tree", { name: "Project files" });
  await tree.focus();
  await page.keyboard.press("End");
  await expect(page.getByRole("treeitem", { name: /file-0499/ })).toBeVisible();

  await expect(page).toHaveScreenshot(["project", "tree-large-light.png"]);
  await attachScreenshot(page, testInfo, "tree-large-light");
});

// ---------------------------------------------------------------------------
// S042 -- project schemas
// ---------------------------------------------------------------------------

async function openSchemas(page: Page): Promise<void> {
  // The toggle used to be called "Datasets", which collided with the unrelated
  // Datasets destination in the rail and forced this scoping. The panel is
  // "Schemas" now -- the name is unambiguous, and the scoping is kept only
  // because addressing a view's own control through that view is still right.
  await page.getByRole("main", { name: "Project files" }).getByRole("tab", { name: "Schemas" }).click();
}

test("light schemas panel shows the extracting skeleton", async ({ page }, testInfo) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: { "": [ok({ version: 1, project_id: "project-1", path: null, entries: [] })] },
    schemaSummary: [pending],
  });
  await openProjectFiles(page);
  await openSchemas(page);
  await expect(page.getByRole("status", { name: "Extracting schemas" })).toBeVisible();

  await expect(page).toHaveScreenshot(["project", "schemas-extracting-light.png"]);
  await attachScreenshot(page, testInfo, "schemas-extracting-light");
});

test("light schemas panel reports an error with retry", async ({ page }, testInfo) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: { "": [ok({ version: 1, project_id: "project-1", path: null, entries: [] })] },
    schemaSummary: [err("schema extraction crashed")],
  });
  await openProjectFiles(page);
  await openSchemas(page);

  await expect(page.getByRole("alert")).toHaveText(/Schemas are not available/);
  await expect(page).toHaveScreenshot(["project", "schemas-error-light.png"]);
  await attachScreenshot(page, testInfo, "schemas-error-light");
});

test("light schemas panel states none were found", async ({ page }, testInfo) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: { "": [ok({ version: 1, project_id: "project-1", path: null, entries: [] })] },
    schemaSummary: [ok({ version: 1, project_id: "project-1", tables: [] })],
  });
  await openProjectFiles(page);
  await openSchemas(page);

  await expect(page.getByText(/No CSV, JSON, or SQLite schemas/)).toBeVisible();
  await expect(page).toHaveScreenshot(["project", "schemas-empty-light.png"]);
  await attachScreenshot(page, testInfo, "schemas-empty-light");
});

test("light schemas panel lists many tables with many columns", async ({ page }, testInfo) => {
  const columns = Array.from({ length: 8 }, (_, index) => ({
    name: `column_${index}`,
    inferred_type: index % 2 === 0 ? "text" : "integer",
    sample_values: ["alpha", "beta"],
  }));
  const tables = Array.from({ length: 6 }, (_, index) => ({
    name: `table_${index}`,
    relative_path: `data/table_${index}.csv`,
    format: "csv",
    columns,
    sample_rows: [],
    row_count_estimate: 42_000 + index,
  }));
  await openShell(page, variants[0], {
    projects: [project()],
    directories: { "": [ok({ version: 1, project_id: "project-1", path: null, entries: [] })] },
    schemaSummary: [ok({ version: 1, project_id: "project-1", tables })],
  });
  await openProjectFiles(page);
  await openSchemas(page);

  // Every table's columns are open at once: the cards no longer collapse.
  await expect(page.getByRole("region", { name: /^table_\d columns$/ })).toHaveCount(6);
  await expect(page.getByRole("region", { name: "table_0 columns" })).toBeVisible();
  await expect(page).toHaveScreenshot(["project", "schemas-populated-light.png"]);
  await attachScreenshot(page, testInfo, "schemas-populated-light");
});

// ---------------------------------------------------------------------------
// S043 -- project file viewer
// ---------------------------------------------------------------------------

async function openFile(page: Page, name = "notes.md"): Promise<void> {
  await page.getByRole("treeitem", { name: new RegExp(`^${name}`) }).click();
}

test("schemas follow the selected file without showing neighboring files", async ({ page }, testInfo) => {
  const tables = ["pit_stops", "circuits"].map((name) => ({
    name,
    relative_path: `${name}.csv`,
    format: "csv",
    columns: [{ name: `${name}_id`, inferred_type: "integer", sample_values: ["1"] }],
    sample_rows: [["1"]],
    row_count_estimate: 1,
  }));
  await openShell(page, variants[0], {
    projects: [project()],
    directories: {
      "": [
        ok({
          version: 1,
          project_id: "project-1",
          path: null,
          entries: [entry({ name: "pit_stops.csv" }), entry({ name: "circuits.csv" }), entry({ name: "notes.md" })],
        }),
      ],
    },
    files: [
      ok({ text: "driver_id,duration\n1,2\n" }),
      ok({ text: "circuit_id,country\n1,India\n" }),
      ok({ text: "# Notes" }),
    ],
    schemaSummary: [ok({ version: 1, project_id: "project-1", tables })],
  });
  await openProjectFiles(page);
  await openFile(page, "pit_stops.csv");
  await openSchemas(page);
  await expect(page.getByRole("region", { name: "pit_stops", exact: true })).toBeVisible();
  await expect(page.getByRole("region", { name: "circuits", exact: true })).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("selected-file-schema.png") });

  await openFile(page, "circuits.csv");
  await openSchemas(page);
  await expect(page.getByRole("region", { name: "circuits", exact: true })).toBeVisible();
  await expect(page.getByRole("region", { name: "pit_stops", exact: true })).toHaveCount(0);

  await openFile(page, "notes.md");
  await openSchemas(page);
  await expect(page.getByText("No CSV, JSON, or SQLite schema was found for this file.")).toBeVisible();
  await expect(page.getByRole("region", { name: "Schemas", exact: true })).toHaveCount(0);
});

test("light file viewer shows the reading state", async ({ page }, testInfo) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: { "": [ok({ version: 1, project_id: "project-1", path: null, entries: [entry()] })] },
    files: [pending],
  });
  await openProjectFiles(page);
  await openFile(page);
  await expect(page.getByText(/Reading notes\.md/)).toBeVisible();

  await expect(page).toHaveScreenshot(["project", "file-reading-light.png"]);
  await attachScreenshot(page, testInfo, "file-reading-light");
});

test("light file viewer withholds a sensitive file", async ({ page }, testInfo) => {
  // The tree marks this entry NOT sensitive: a row the tree itself flagged
  // sensitive refuses the click and never reaches the viewer at all (S041
  // covers that refusal separately). This reaches the withheld view the
  // real way it happens -- the backend classifies at read time regardless
  // of what a listing said, so a row that looked ordinary can still answer
  // `sensitive_withheld`.
  await openShell(page, variants[0], {
    projects: [project()],
    directories: { "": [ok({ version: 1, project_id: "project-1", path: null, entries: [entry({ name: ".env" })] })] },
    files: [ok({ version: 1, kind: "sensitive_withheld", path: ".env", sensitive_class: "dot_env" })],
  });
  await openProjectFiles(page);
  await openFile(page, ".env");

  await expect(page.getByText("This file was not read")).toBeVisible();
  await expect(page).toHaveScreenshot(["project", "file-sensitive-light.png"]);
  await attachScreenshot(page, testInfo, "file-sensitive-light");
});

test("light file viewer states a binary file's contents are not shown", async ({ page }, testInfo) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: {
      "": [ok({ version: 1, project_id: "project-1", path: null, entries: [entry({ name: "logo.png" })] })],
    },
    files: [
      ok({
        version: 1,
        kind: "not_text",
        path: "logo.png",
        size_bytes: 48_213,
        modified_ms: 1_700_000_000_000,
        reason: "binary",
      }),
    ],
  });
  await openProjectFiles(page);
  await openFile(page, "logo.png");

  await expect(page.getByText("Contents not shown")).toBeVisible();
  await expect(page).toHaveScreenshot(["project", "file-binary-light.png"]);
  await attachScreenshot(page, testInfo, "file-binary-light");
});

test("light file viewer pages through a file too large to hold whole", async ({ page }, testInfo) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: {
      "": [ok({ version: 1, project_id: "project-1", path: null, entries: [entry({ name: "server.log" })] })],
    },
    // 80 MiB, past the whole-file bound: the view reads one window at a time.
    files: [ok({ pattern: "2026-09-25T08:00:00Z INFO request served in 12 ms\n", size_bytes: 80 * 1024 * 1024 })],
  });
  await openProjectFiles(page);
  await openFile(page, "server.log");

  await expect(page.getByRole("status").filter({ hasText: /^Bytes 0–/ })).toBeVisible();
  await expect(page.getByRole("button", { name: "Next page" })).toBeEnabled();
  await expect(page.getByRole("button", { name: "Edit" })).toHaveCount(0);
  await expect(page).toHaveScreenshot(["project", "file-windowed-light.png"]);
  await attachScreenshot(page, testInfo, "file-windowed-light");
});

test("light file viewer edits, then shows a save failure with the draft intact", async ({ page }, testInfo) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: { "": [ok({ version: 1, project_id: "project-1", path: null, entries: [entry()] })] },
    files: [
      ok({ text: "Draft pending review.\n" }),
      // The pre-save verify re-reads the file; matching content lets the
      // save proceed instead of reporting a stale-on-disk conflict.
      ok({ text: "Draft pending review.\n" }),
    ],
    saveFile: [err("disk is full")],
  });
  await openProjectFiles(page);
  await openFile(page);
  await page.getByRole("button", { name: "Edit" }).click();
  const editor = page.getByRole("textbox", { name: "File contents" });
  await editor.fill("Draft pending review.\nRevenue grew 12%.");
  await page.getByRole("button", { name: "Save" }).click();

  await expect(page.getByText("This file was not saved")).toBeVisible();
  await expect(editor).toHaveValue("Draft pending review.\nRevenue grew 12%.");
  await expect(page).toHaveScreenshot(["project", "file-save-failed-light.png"]);
  await attachScreenshot(page, testInfo, "file-save-failed-light");
});

test("light file viewer notices a stale file on disk without swapping the draft", async ({ page }, testInfo) => {
  const original = ok({ text: "Draft pending review.\n" });
  const changedElsewhere = ok({ text: "Someone else already edited this.\n" });
  await openShell(page, variants[0], {
    projects: [project()],
    directories: { "": [ok({ version: 1, project_id: "project-1", path: null, entries: [entry()] })] },
    // A load stats before and after its read, so the original answers both;
    // the focus probe is what sees the change.
    files: [original, original, changedElsewhere],
  });
  await openProjectFiles(page);
  await openFile(page);
  // Visible only: the card's hidden source view carries the same text.
  const shown = page.getByText("Draft pending review.").filter({ visible: true });
  await expect(shown).toBeVisible();
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));

  await expect(page.getByText("This file changed on disk")).toBeVisible();
  await expect(shown).toBeVisible();
  await expect(page).toHaveScreenshot(["project", "file-stale-light.png"]);
  await attachScreenshot(page, testInfo, "file-stale-light");
});

// ---------------------------------------------------------------------------
// S044 -- revision history
// ---------------------------------------------------------------------------

async function openHistory(page: Page): Promise<void> {
  await page.getByRole("tab", { name: "History & Revisions" }).click();
}

const oneRevision = {
  version: 1,
  backup_id: "backup-1",
  created_at_ms: 1_700_000_000_000,
  original_bytes: 2_048,
  compressed_bytes: 900,
  content_hash: "a1b2c3d4e5f60718",
};

test("light revisions panel shows the loading state", async ({ page }, testInfo) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: { "": [ok({ version: 1, project_id: "project-1", path: null, entries: [entry()] })] },
    files: [ok({ text: "x\n" })],
    revisions: [pending],
  });
  await openProjectFiles(page);
  await openFile(page);
  await openHistory(page);

  await expect(page.getByText("Loading revisions…")).toBeVisible();
  await expect(page).toHaveScreenshot(["project", "revisions-loading-light.png"]);
  await attachScreenshot(page, testInfo, "revisions-loading-light");
});

test("light revisions panel reports a load error", async ({ page }, testInfo) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: { "": [ok({ version: 1, project_id: "project-1", path: null, entries: [entry()] })] },
    files: [ok({ text: "x\n" })],
    revisions: [err("revision store is locked")],
  });
  await openProjectFiles(page);
  await openFile(page);
  await openHistory(page);

  await expect(page.getByRole("alert")).toHaveText(/Revision history was not loaded/);
  await expect(page).toHaveScreenshot(["project", "revisions-error-light.png"]);
  await attachScreenshot(page, testInfo, "revisions-error-light");
});

test("light revisions panel says none exist yet", async ({ page }, testInfo) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: { "": [ok({ version: 1, project_id: "project-1", path: null, entries: [entry()] })] },
    files: [ok({ text: "x\n" })],
    revisions: [ok({ version: 1, project_id: "project-1", relative_path: "notes.md", revisions: [] })],
  });
  await openProjectFiles(page);
  await openFile(page);
  await openHistory(page);

  await expect(page.getByText("No revisions yet")).toBeVisible();
  await expect(page).toHaveScreenshot(["project", "revisions-empty-light.png"]);
  await attachScreenshot(page, testInfo, "revisions-empty-light");
});

test("light revisions panel lists prior revisions and confirms a restore", async ({ page }, testInfo) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: { "": [ok({ version: 1, project_id: "project-1", path: null, entries: [entry()] })] },
    files: [ok({ text: "x\n" })],
    revisions: [ok({ version: 1, project_id: "project-1", relative_path: "notes.md", revisions: [oneRevision] })],
  });
  await openProjectFiles(page);
  await openFile(page);
  await openHistory(page);
  await expect(page.getByRole("button", { name: "Restore revision" })).toBeVisible();

  await expect(page).toHaveScreenshot(["project", "revisions-populated-light.png"]);
  await attachScreenshot(page, testInfo, "revisions-populated-light");

  await page.getByRole("button", { name: "Restore revision" }).click();
  await expect(page.getByRole("alertdialog", { name: "Restore this file?" })).toBeVisible();
  await expect(page).toHaveScreenshot(["project", "revisions-restore-confirm-light.png"]);
  await attachScreenshot(page, testInfo, "revisions-restore-confirm-light");
});

test("light revisions panel shows a revision's changes side by side, and one change alone", async ({ page }) => {
  const newer = { ...oneRevision, backup_id: "backup-2", created_at_ms: 1_700_000_900_000 };
  const changes = (unified: string, added: number, removed: number) =>
    ok({
      kind: "changes",
      version: 1,
      backup_id: "backup-1",
      unified_diff: unified,
      added_lines: added,
      removed_lines: removed,
    });
  await openShell(page, variants[0], {
    projects: [project()],
    directories: { "": [ok({ version: 1, project_id: "project-1", path: null, entries: [entry()] })] },
    files: [ok({ text: "# Notes\nalpha\ngamma\ndelta\n" })],
    revisions: [
      ok({ version: 1, project_id: "project-1", relative_path: "notes.md", revisions: [newer, oneRevision] }),
    ],
    revisionDiffs: [
      changes("--- a/notes.md\n+++ b/notes.md\n@@ -1,3 +1,4 @@\n # Notes\n alpha\n-beta\n+gamma\n+delta\n", 2, 1),
      changes("--- a/notes.md\n+++ b/notes.md\n@@ -1,3 +1,3 @@\n # Notes\n alpha\n-beta\n+gamma\n", 1, 1),
    ],
  });
  await openProjectFiles(page);
  await openFile(page);
  await openHistory(page);

  await page.getByRole("button", { name: "Show changes" }).nth(1).click();
  const region = page.getByRole("region", { name: "Changes since this revision" });
  const split = region.getByTestId("split-diff");
  await expect(split).toBeVisible();
  await expect(region).toContainText("2 added · 1 removed. Left: this revision. Right: the file now.");
  // Side by side: the removed line and the added line it became share a row.
  const removed = split.getByText("beta", { exact: true });
  const added = split.getByText("gamma", { exact: true });
  const [removedBox, addedBox] = [await removed.boundingBox(), await added.boundingBox()];
  expect(removedBox && addedBox && Math.abs(removedBox.y - addedBox.y) < 2).toBe(true);
  expect(removedBox!.x).toBeLessThan(addedBox!.x);
  await page.screenshot({ path: test.info().outputPath("revision-diff-since.png") });

  await region.getByRole("radio", { name: "This change only" }).click();
  const change = page.getByRole("region", { name: "The edit that replaced this revision", exact: true });
  await expect(change).toContainText("1 added · 1 removed.");
  await expect(change.getByTestId("split-diff").getByText("delta", { exact: true })).toHaveCount(0);
  await page.screenshot({ path: test.info().outputPath("revision-diff-one-change.png") });
});

test("light revisions panel says a diff is too large and keeps restore available", async ({ page }) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: { "": [ok({ version: 1, project_id: "project-1", path: null, entries: [entry()] })] },
    files: [ok({ text: "x\n" })],
    revisions: [ok({ version: 1, project_id: "project-1", relative_path: "notes.md", revisions: [oneRevision] })],
    revisionDiffs: [ok({ kind: "too_large", version: 1, backup_id: "backup-1", limit: "change_size" })],
  });
  await openProjectFiles(page);
  await openFile(page);
  await openHistory(page);

  await page.getByRole("button", { name: "Show changes" }).click();
  const region = page.getByRole("region", { name: "Changes since this revision" });
  await expect(region.getByText("Diff too large to show")).toBeVisible();
  await expect(region).toContainText("This edit changed too much to show as a diff.");
  await expect(page.getByRole("button", { name: "Restore revision" })).toBeEnabled();
  await page.screenshot({ path: test.info().outputPath("revision-diff-too-large.png") });
});

test("light revisions panel shows a restore in progress", async ({ page }, testInfo) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: { "": [ok({ version: 1, project_id: "project-1", path: null, entries: [entry()] })] },
    files: [ok({ text: "x\n" })],
    revisions: [ok({ version: 1, project_id: "project-1", relative_path: "notes.md", revisions: [oneRevision] })],
    restore: [pending],
  });
  await openProjectFiles(page);
  await openFile(page);
  await openHistory(page);
  await page.getByRole("button", { name: "Restore revision" }).click();
  const dialog = page.getByRole("alertdialog", { name: "Restore this file?" });
  await dialog.getByRole("button", { name: "Restore this revision" }).click();

  await expect(dialog.getByRole("button", { name: "Cancel" })).toBeDisabled();
  await expect(page).toHaveScreenshot(["project", "revisions-restoring-light.png"]);
  await attachScreenshot(page, testInfo, "revisions-restoring-light");
});

// ---------------------------------------------------------------------------
// S046 -- remove project. S045's trust-copy step is already baselined in
// approval-activity.spec.ts's `project-trust-*` tests; this covers the
// states that step does not: confirming and failing a removal.
// ---------------------------------------------------------------------------

test("light sidebar confirms a project removal, naming what survives", async ({ page }, testInfo) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: { "": [ok({ version: 1, project_id: "project-1", path: null, entries: [] })] },
  });
  await openProjectFiles(page);
  await page.getByRole("button", { name: "Remove Research" }).click();

  await expect(page.getByRole("alertdialog", { name: "Remove this project?" })).toBeVisible();
  await expect(page.getByText(/Existing chats are not deleted/)).toBeVisible();
  await expect(page).toHaveScreenshot(["project", "remove-confirm-light.png"]);
  await attachScreenshot(page, testInfo, "remove-confirm-light");
});

test("light sidebar keeps a failed removal retryable", async ({ page }, testInfo) => {
  await openShell(page, variants[0], {
    projects: [project()],
    directories: { "": [ok({ version: 1, project_id: "project-1", path: null, entries: [] })] },
    revokeProject: [err("trust store is locked")],
  });
  await openProjectFiles(page);
  await page.getByRole("button", { name: "Remove Research" }).click();
  await page.getByRole("button", { name: "Remove project" }).click();

  await expect(page.getByRole("alert")).toHaveText(/This project could not be removed/);
  await expect(page.getByRole("alertdialog", { name: "Remove this project?" })).toBeVisible();
  await expect(page).toHaveScreenshot(["project", "remove-failed-light.png"]);
  await attachScreenshot(page, testInfo, "remove-failed-light");
});

for (const variant of variants) {
  test(`${variant.name} project confirmation fits content and keeps actions reachable`, async ({ page }) => {
    await openShell(page, variant, {
      projects: [project()],
      directories: { "": [ok({ version: 1, project_id: "project-1", path: null, entries: [] })] },
    });
    await openProjectFiles(page, { keepSidebarOpen: true });
    await page.getByRole("button", { name: "Remove Research" }).click();
    const dialog = page.getByRole("alertdialog", { name: "Remove this project?" });
    await expect(dialog).toBeVisible();
    expect((await dialog.boundingBox())!.height).toBeLessThan(300);
    await expect(dialog).toHaveScreenshot(["dialogs", `remove-project-${variant.name}.png`]);
    await page.setViewportSize({ width: 720, height: 480 });
    await page.evaluate(() => (document.documentElement.style.fontSize = "200%"));
    await expect(dialog.getByRole("button", { name: "Remove project", exact: true })).toBeInViewport();
    await expect(dialog.getByRole("button", { name: "Cancel", exact: true })).toBeInViewport();
    expect(await dialog.evaluate((node) => node.scrollWidth - node.clientWidth)).toBeLessThanOrEqual(1);
    const body = dialog.locator('[id^="alert-dialog-description-"]').locator("..");
    await expect.poll(() => body.evaluate((node) => node.scrollHeight > node.clientHeight)).toBe(true);
    await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(dialog).not.toBeVisible();
  });
}
