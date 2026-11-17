import { expect, test, type Page, type TestInfo } from "@playwright/test";

// ---------------------------------------------------------------------------
// Covers the Task 8 slice's genuinely-reachable visual states:
//
// - "project trust": `ProjectCreateSheet`, opened for real through the
//   Projects destination's own "New project" item, showing the trust
//   explanation before any folder is trusted.
// - "expanded activity": `LiveRunActivity`, expanded for real by clicking
//   its trigger.
//
// No file-patch review state: an agent's project change applies without one
// (ADR-043).
//
// Deliberately NOT covered here, and not fabricated: a "sensitive-read
// override" state. No sensitive-read frontend surface exists at all -- an
// agent cannot request an override and there is nothing for a person to
// answer.
//
// Mirrors `shell.spec.ts`'s structure: same variant set, same
// native-boundary mocking pattern via `page.addInitScript`.
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
  {
    name: "light",
    theme: "light",
    viewport: { width: 1440, height: 960 },
  },
  {
    name: "dark",
    theme: "dark",
    viewport: { width: 1440, height: 960 },
  },
  {
    name: "narrow",
    theme: "light",
    viewport: { width: 720, height: 900 },
  },
  {
    name: "high-contrast",
    theme: "light",
    viewport: { width: 1440, height: 960 },
    contrast: "more",
  },
];

async function installNativeBoundaryMock(page: Page, variant: VisualVariant): Promise<void> {
  await page.addInitScript(
    ({ mockBootstrap, theme }) => {
      const callbacks = new Map<number, (data: unknown) => void>();
      const listeners = new Map<string, number[]>();
      let nextCallback = 1;
      const transformCallback = (callback: (data: unknown) => void, once = false) => {
        const id = nextCallback++;
        callbacks.set(id, (data) => {
          callback(data);
          if (once) callbacks.delete(id);
        });
        return id;
      };
      const emit = (event: string, payload: unknown) => {
        for (const id of listeners.get(event) ?? []) callbacks.get(id)?.({ event, payload });
      };
      let approvalMode: "resolve" | "pending" = "resolve";
      let pendingApproval: { resolve: () => void; reject: (cause: Error) => void } | null = null;
      Object.assign(window, {
        __VISUAL_EMIT__: emit,
        __VISUAL_LAST_TOOL_APPROVAL__: null,
        __VISUAL_SET_APPROVAL_MODE__: (mode: "resolve" | "pending") => {
          approvalMode = mode;
        },
        __VISUAL_SETTLE_APPROVAL__: (outcome: "resolve" | "reject") => {
          const pending = pendingApproval;
          pendingApproval = null;
          if (!pending) throw new Error("No tool approval is pending");
          if (outcome === "resolve") pending.resolve();
          else pending.reject(new Error("that request could not be resolved"));
        },
      });

      Object.defineProperty(window, "__TAURI_INTERNALS__", {
        configurable: false,
        value: Object.freeze({
          transformCallback,
          invoke(command: string, args?: Record<string, unknown>) {
            if (command === "plugin:event|listen") {
              const event = String(args?.event);
              const handler = Number(args?.handler);
              listeners.set(event, [...(listeners.get(event) ?? []), handler]);
              return Promise.resolve(handler);
            }
            if (command === "plugin:event|unlisten") {
              const event = String(args?.event);
              const eventId = Number(args?.eventId);
              listeners.set(
                event,
                (listeners.get(event) ?? []).filter((id) => id !== eventId),
              );
              return Promise.resolve(null);
            }
            if (command === "app_bootstrap") return Promise.resolve(mockBootstrap);
            if (command === "app_health") {
              return Promise.resolve({ version: 1, status: "ready" });
            }
            if (command === "app_reset") return Promise.resolve(undefined);
            if (command === "settings_appearance_get") {
              return Promise.resolve({
                version: 1,
                theme,
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
              return Promise.resolve({
                version: 1,
                conversations: [
                  {
                    version: 1,
                    conversation_id: "conversation-visual-1",
                    title: "Visual activity",
                    updated_at: "2026-08-14T12:00:00Z",
                    pinned_at: null,
                    project_id: null,
                  },
                ],
                recovery_notice: null,
              });
            }
            if (command === "chat_conversation_open") {
              return Promise.resolve({
                version: 1,
                conversation_id: "conversation-visual-1",
                title: "Visual activity",
                availability: "live",
                messages: [
                  {
                    version: 1,
                    message_id: "message-visual-1",
                    role: "user",
                    text: "Review the latest warehouse results.",
                    sequence: BigInt(1),
                    tool_calls: [],
                  },
                ],
                active_run_id: "run-visual-1",
              });
            }
            if (command === "chat_message_feedback_list") return Promise.resolve([]);
            if (command === "chat_tool_approval_resolve") {
              Object.assign(window, { __VISUAL_LAST_TOOL_APPROVAL__: args ?? null });
              if (approvalMode === "resolve") return Promise.resolve(null);
              return new Promise<void>((resolve, reject) => {
                pendingApproval = { resolve, reject };
              });
            }
            // No project has been trusted yet -- the switcher starts empty, so
            // "New project" opens the shared trust flow, same as a
            // real first run.
            if (command === "project_list") return Promise.resolve({ version: 1, projects: [] });
            // The OS directory picker (`@tauri-apps/plugin-dialog`'s `open()`)
            // goes through this command, not a `project_*` one.
            if (command === "plugin:dialog|open") return Promise.resolve("/home/user/research-notes");
            if (command === "project_find_by_folder") return Promise.resolve({ version: 1, projects: [] });
            if (command === "project_create") {
              const path = typeof args?.path === "string" ? args.path : "/home/user/research-notes";
              const displayName = typeof args?.displayName === "string" ? args.displayName : "research-notes";
              return Promise.resolve({
                version: 1,
                id: "project-visual-1",
                display_name: displayName,
                root_path: path,
                status: "trusted",
                active: true,
                trusted_at_ms: Date.now(),
              });
            }
            return Promise.reject(new Error("visual test rejected an unknown native command"));
          },
        }),
        writable: false,
      });
    },
    { mockBootstrap: bootstrap, theme: variant.theme },
  );
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

/** `ProjectSwitcher` lives inside the collapsible sidebar, which starts
 * closed on a narrow viewport (see `shell.spec.ts`'s own narrow-variant
 * assertion) -- open it first so the trigger is actually reachable. A no-op
 * on wider variants, where the sidebar is already open and this button reads
 * "Hide sidebar" instead. */
async function ensureSidebarOpen(page: Page): Promise<void> {
  const showSidebar = page.getByRole("button", { name: "Show sidebar" });
  if (await showSidebar.isVisible().catch(() => false)) {
    await showSidebar.click();
  }
}

async function attachScreenshot(page: Page, testInfo: TestInfo, name: string): Promise<void> {
  const screenshotPath = testInfo.outputPath(`${name}.png`);
  await page.screenshot({
    animations: "disabled",
    caret: "hide",
    fullPage: false,
    path: screenshotPath,
    scale: "css",
  });
  await testInfo.attach(`${name}.png`, {
    path: screenshotPath,
    contentType: "image/png",
  });
}

for (const variant of variants) {
  test(`${variant.name} project trust sheet explains trust before acceptance`, async ({ page }, testInfo) => {
    await openShell(page, variant);
    await ensureSidebarOpen(page);

    // Chats and Projects own one sidebar column each, so the create action
    // lives on the Projects destination rather than beside the chat list.
    await page.getByRole("button", { name: "Projects" }).click();
    await expect(page.getByRole("main", { name: "Project files" })).toBeVisible();
    // A narrow window carries the panel as a drawer, and choosing a
    // destination closes it. The create action needs it open again.
    await ensureSidebarOpen(page);

    await page.getByRole("navigation", { name: "Workspace" }).getByRole("button", { name: "New project" }).click();

    const dialog = page.getByRole("dialog", { name: "New project" });
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "Choose folder" }).click();

    await expect(dialog.getByText(/Trusting this folder/)).toBeVisible();
    await expect(dialog.getByText("/home/user/research-notes")).toBeVisible();
    await expect(dialog.getByText(/without asking first/)).toBeVisible();
    await expect(dialog.getByText(/permanent deletion of this folder/)).toBeVisible();
    await expect(dialog.getByRole("button", { name: "Trust & create project" })).toBeVisible();

    // Opening the trust sheet must never introduce any of the surfaces this
    // app deliberately omits.
    await expect(page.getByText("MCP", { exact: true })).toHaveCount(0);
    await expect(page.getByText("Skills", { exact: true })).toHaveCount(0);

    await expect(page).toHaveScreenshot(["approval-activity", `project-trust-${variant.name}.png`]);
    await attachScreenshot(page, testInfo, `project-trust-${variant.name}`);
  });

  test(`${variant.name} live activity expands with the in-flight run`, async ({ page }, testInfo) => {
    await openShell(page, variant);
    await ensureSidebarOpen(page);
    await page.getByRole("option", { name: "Visual activity" }).click();
    await expect(page.getByText("Review the latest warehouse results.")).toBeVisible();
    await page.evaluate(() => {
      const emit = (window as Window & { __VISUAL_EMIT__?: (event: string, payload: unknown) => void }).__VISUAL_EMIT__;
      emit?.("xfalcon://chat-projection-event", {
        type: "run_state_changed",
        version: 1,
        conversation_id: "conversation-visual-1",
        run_id: "run-visual-1",
        state: "running",
      });
      emit?.("xfalcon://tool-invocation-event", {
        type: "requested",
        version: 1,
        invocation_id: "invocation-visual-1",
        run_id: "run-visual-1",
        tool_id: "mcp.warehouse.query",
      });
    });

    // Every phrase in `THINKING_PHRASES`, not just one of them. A working
    // block is named by `useThinkingPhrase`, which leads with "Thinking" and
    // rotates every `PHRASE_HOLD_MS`, so naming a single phrase here matched
    // nothing once the lead changed — and would have been a flake even before
    // that, since which phrase is showing depends on when the assertion runs.
    const trigger = page.getByRole("button", {
      name: /Thinking|Working|Reviewing the context…|Preparing your response…|Putting it together…/,
    });
    await expect(trigger).toHaveAttribute("aria-expanded", "true");
    await expect(page.getByText("query", { exact: true })).toBeVisible();
    await expect(page.getByText("· warehouse", { exact: true })).toBeVisible();

    await expect(page).toHaveScreenshot(["approval-activity", `activity-expanded-${variant.name}.png`]);
    await attachScreenshot(page, testInfo, `activity-expanded-${variant.name}`);
  });
}

test("tool approval stays in the run tail and its busy failure path remains keyboard-recoverable", async ({ page }) => {
  await openShell(page, variants[0]);
  await page.getByRole("option", { name: "Visual activity" }).click();
  await page.evaluate(() => {
    const fixture = window as Window & {
      __VISUAL_EMIT__?: (event: string, payload: unknown) => void;
      __VISUAL_SET_APPROVAL_MODE__?: (mode: "resolve" | "pending") => void;
    };
    fixture.__VISUAL_SET_APPROVAL_MODE__?.("pending");
    fixture.__VISUAL_EMIT__?.("xfalcon://chat-projection-event", {
      type: "run_state_changed",
      version: 1,
      conversation_id: "conversation-visual-1",
      run_id: "run-visual-1",
      state: "awaiting_approval",
    });
    fixture.__VISUAL_EMIT__?.("xfalcon://tool-invocation-event", {
      type: "requested",
      version: 1,
      invocation_id: "invocation-visual-1",
      run_id: "run-visual-1",
      tool_id: "mcp.warehouse.write_rows",
    });
    fixture.__VISUAL_EMIT__?.("xfalcon://tool-invocation-event", {
      type: "approval_required",
      version: 1,
      invocation_id: "invocation-visual-1",
      tool_id: "mcp.warehouse.write_rows",
    });
  });

  const approval = page.getByRole("region", { name: /Approval required for write_rows/ });
  await expect(approval).toBeVisible();
  expect(await approval.evaluate((node) => node.closest(".chat-column-messages") !== null)).toBe(true);
  await expect(page.getByText("Awaiting approval")).toBeVisible();

  const allow = approval.getByRole("button", { name: "Allow once" });
  await allow.focus();
  await page.keyboard.press("Enter");
  await expect(approval.getByRole("button", { name: "Allowing…" })).toHaveAttribute("aria-busy", "true");
  await expect(approval.getByRole("button", { name: "Deny" })).toBeDisabled();

  await page.evaluate(() => {
    const fixture = window as Window & { __VISUAL_SETTLE_APPROVAL__?: (outcome: "resolve" | "reject") => void };
    fixture.__VISUAL_SETTLE_APPROVAL__?.("reject");
  });
  await expect(approval.getByRole("alert")).toContainText("The decision could not be saved");
  await expect(allow).toBeFocused();

  await page.evaluate(() => {
    const fixture = window as Window & {
      __VISUAL_SET_APPROVAL_MODE__?: (mode: "resolve" | "pending") => void;
    };
    fixture.__VISUAL_SET_APPROVAL_MODE__?.("resolve");
  });
  const deny = approval.getByRole("button", { name: "Deny" });
  await deny.focus();
  await page.keyboard.press("Enter");
  await expect(approval.getByRole("button", { name: "Denied" })).toBeDisabled();
  expect(
    await page.evaluate(
      () => (window as Window & { __VISUAL_LAST_TOOL_APPROVAL__?: unknown }).__VISUAL_LAST_TOOL_APPROVAL__,
    ),
  ).toEqual({ runId: "run-visual-1", invocationId: "invocation-visual-1", approve: false });

  await page.evaluate(() => {
    (window as Window & { __VISUAL_EMIT__?: (event: string, payload: unknown) => void }).__VISUAL_EMIT__?.(
      "xfalcon://tool-invocation-event",
      {
        type: "denied",
        version: 1,
        invocation_id: "invocation-visual-1",
        reason_code: "user_declined",
      },
    );
  });
  await expect(approval).toHaveCount(0);
  await expect(page.getByText("Denied", { exact: true })).toBeVisible();
});
