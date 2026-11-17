import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppBootstrap } from "../lib/generated/contracts";
import { openCanvasArtifact, resetCanvasStore } from "../features/canvas/canvasStore";
import { resetAuthSessionForTests, setAuthSession } from "../features/auth/authStore";
import { resetDatasetsStore } from "../features/datasets/datasetsStore";
import { resetReportsStore } from "../features/reports/reportsStore";
import { resetChatStore, stopChatEventBridge } from "../features/chat/chatStore";
import { AppShell } from "./AppShell";
import { AppErrorBoundary } from "./AppErrorBoundary";
import { preloadWorkspaceViews } from "./lazyViews";

/*
 * The lazy views' chunks are held behind gates. A gate stays closed until the
 * test opens it, so the test can see the previous view stay on screen while the
 * new one is still loading. A module's chunk is fetched once per file, so each
 * gate is opened (or failed) by the test that needs it, and the gate that fails
 * is one nothing else in this file renders.
 */
const chunkGates = vi.hoisted(() => {
  const requested = new Set<string>();
  const gate = (name: string) => {
    let open!: () => void;
    let fail!: (error: Error) => void;
    const promise = new Promise<void>((resolve, reject) => {
      open = resolve;
      fail = reject;
    });
    // A gate can fail before anything has asked for it; the failure is still
    // reported where the view renders, not as an unhandled rejection here.
    promise.catch(() => undefined);
    return {
      open,
      fail,
      request: () => {
        requested.add(name);
        return promise;
      },
    };
  };
  return {
    requested,
    settings: gate("settings"),
    projects: gate("projects"),
    datasets: gate("datasets"),
    reports: gate("reports"),
    canvas: gate("canvas"),
    export: gate("export"),
  };
});

vi.mock("../features/settings/SettingsView", async (importOriginal) => {
  await chunkGates.settings.request();
  return importOriginal();
});
vi.mock("../features/projects/ProjectExplorerView", async (importOriginal) => {
  await chunkGates.projects.request();
  return importOriginal();
});
vi.mock("../features/datasets/DatasetsView", async (importOriginal) => {
  await chunkGates.datasets.request();
  return importOriginal();
});
vi.mock("../features/reports/ReportsView", async (importOriginal) => {
  await chunkGates.reports.request();
  return importOriginal();
});
vi.mock("../features/canvas/CanvasPanel", async (importOriginal) => {
  await chunkGates.canvas.request();
  return importOriginal();
});
vi.mock("../features/chat/ChatExportDialog", async (importOriginal) => {
  await chunkGates.export.request();
  return importOriginal();
});
vi.mock("./lazyViews", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./lazyViews")>();
  return { ...actual, preloadWorkspaceViews: vi.fn(actual.preloadWorkspaceViews) };
});

const invokeMock = vi.hoisted(() => vi.fn());
const listenMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  convertFileSrc: (id: string) => `http://xf-preview.localhost/${id}`,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

const bootstrap: AppBootstrap = {
  version: 1,
  application_version: "0.1.0",
  platform: "windows",
  session: {
    status: "authenticated",
    version: 1,
    identity: {
      version: 1,
      user_id: "user-test",
      tenant_id: "tenant-test",
      display_name: "Test User",
      email: null,
    },
    expires_at_ms: Date.now() + 3_600_000,
  },
};

function mockNativeBoundary() {
  invokeMock.mockImplementation(async (command: string) => {
    if (command === "workspace_status") {
      return {
        version: 1,
        tenant_host: "tenant.example",
        mcp_resource_url: "https://tenant.example/mcp",
        mcp_servers: [],
        models_available: 1,
        default_model_id: "gpt-test",
        default_provider: "openai",
        setup_note: "Bundled MCP is automatic.",
        chat_ready: true,
      };
    }
    if (command === "chat_workspace_bootstrap") {
      return { version: 1, conversations: [], recovery_notice: null };
    }
    if (command === "chat_conversation_preview") return null;
    if (command === "chat_conversation_reset_active") return undefined;
    if (command === "project_find_by_folder") return { version: 1, projects: [] };
    if (command === "project_list") return { version: 1, projects: [] };
    if (command === "credential_list") return [];
    if (command === "mcp_server_list") return { version: 1, servers: [] };
    if (command === "report_list") return { version: 1, reports: [] };
    if (command === "dataset_list") {
      return { version: 1, datasets: [], uploads_enabled: true, truncated: false };
    }
    if (command === "skill_list") return { version: 1, skills: [] };
    if (command === "project_list_directory") return { version: 1, entries: [] };
    throw new Error(`Unexpected invoke: ${command}`);
  });
}

afterEach(async () => {
  cleanup();
  await stopChatEventBridge();
  resetAuthSessionForTests();
  vi.unstubAllGlobals();
});

beforeEach(() => {
  resetChatStore();
  resetDatasetsStore();
  resetReportsStore();
  resetCanvasStore();
  invokeMock.mockReset();
  listenMock.mockReset();
  listenMock.mockResolvedValue(() => {});
  mockNativeBoundary();
  setAuthSession(bootstrap.session);
  vi.mocked(preloadWorkspaceViews).mockClear();
});

/*
 * The preload waits 300 ms on window.setTimeout. Real timers would race the
 * render, so 300 ms timers are captured instead of scheduled: the test fires them
 * through fireDue, inside act, when it decides. A cleared timer never fires, as
 * with real timers. Timers with any other delay run for real. The caller
 * restores both spies.
 */
function captureWorkspacePreloadTimer() {
  const realSetTimeout = window.setTimeout;
  const realClearTimeout = window.clearTimeout;
  const scheduled: Array<{ id: number; run: () => void; cleared: boolean; ran: boolean }> = [];
  let nextId = 1_000_000;
  type TimerId = ReturnType<typeof window.setTimeout>;
  const setTimeoutSpy = vi.spyOn(window, "setTimeout").mockImplementation((handler, timeout, ...rest): TimerId => {
    if (timeout !== 300) return realSetTimeout.call(window, handler, timeout, ...rest);
    const id = nextId++;
    scheduled.push({ id, run: handler as () => void, cleared: false, ran: false });
    return id as unknown as TimerId;
  });
  const clearTimeoutSpy = vi.spyOn(window, "clearTimeout").mockImplementation((id) => {
    const timer = scheduled.find((recorded) => recorded.id === id);
    if (timer != null) {
      timer.cleared = true;
      return;
    }
    realClearTimeout.call(window, id);
  });
  return {
    scheduled,
    clearTimeoutSpy,
    fireDue: () => {
      for (const timer of scheduled) {
        if (timer.cleared || timer.ran) continue;
        timer.ran = true;
        timer.run();
      }
    },
    restore: () => {
      setTimeoutSpy.mockRestore();
      clearTimeoutSpy.mockRestore();
    },
  };
}

// Runs first on purpose: the chunk requests it counts are the first ones this
// file makes, so nothing earlier can have requested them already.
describe("AppShell lazy views", () => {
  it("requests every lazy view once, 300 ms after the shell first becomes interactive", async () => {
    const timer = captureWorkspacePreloadTimer();
    try {
      render(<AppShell bootstrap={bootstrap} />);
      await screen.findByRole("button", { name: "Settings" });
      await vi.waitFor(() => expect(timer.scheduled.length).toBeGreaterThan(0));
      expect(preloadWorkspaceViews).not.toHaveBeenCalled();

      act(() => timer.fireDue());
      expect(preloadWorkspaceViews).toHaveBeenCalledTimes(1);
      await vi.waitFor(() =>
        expect([...chunkGates.requested].sort()).toEqual([
          "canvas",
          "datasets",
          "export",
          "projects",
          "reports",
          "settings",
        ]),
      );

      // Drop the identity and restore it: the effect runs again with the shell
      // interactive, and the latch must stop it from scheduling a second preload.
      // If the latch's check were removed, that second timer would be captured
      // and fireDue would run it, making the count 2 and failing this assertion.
      act(() => setAuthSession({ status: "unauthenticated", version: 1 }));
      act(() => setAuthSession(bootstrap.session));
      await screen.findByRole("button", { name: "Settings" });
      act(() => timer.fireDue());
      expect(preloadWorkspaceViews).toHaveBeenCalledTimes(1);
    } finally {
      timer.restore();
    }
  });

  it("cancels the pending preload when the shell unmounts before it fires", async () => {
    const timer = captureWorkspacePreloadTimer();
    try {
      const { unmount } = render(<AppShell bootstrap={bootstrap} />);
      await screen.findByRole("button", { name: "Settings" });
      await vi.waitFor(() => expect(timer.scheduled.length).toBeGreaterThan(0));
      unmount();

      act(() => timer.fireDue());
      expect(preloadWorkspaceViews).not.toHaveBeenCalled();
    } finally {
      timer.restore();
    }
  });

  it.each([
    { destination: "Settings", chunk: "settings", view: "Settings", highlight: false },
    { destination: "Datasets", chunk: "datasets", view: "Datasets", highlight: true },
    { destination: "Reports", chunk: "reports", view: "Reports", highlight: true },
    { destination: "Projects", chunk: "projects", view: "Project files", highlight: true },
  ] as const)(
    "keeps the previous view on screen until the $destination chunk resolves",
    async ({ destination, chunk, view, highlight }) => {
      const user = userEvent.setup();
      render(<AppShell bootstrap={bootstrap} />);
      await screen.findByRole("main", { name: "Chat workspace" });

      await user.click(screen.getByRole("button", { name: destination }));

      // The sidebar selection moves at once; the view waits for its chunk.
      // Settings is highlighted from the view itself, not the selection.
      if (highlight) {
        expect(screen.getByRole("button", { name: destination })).toHaveAttribute("aria-current", "page");
      }
      expect(screen.getByRole("main", { name: "Chat workspace" })).toBeInTheDocument();
      expect(screen.queryByRole("main", { name: view })).not.toBeInTheDocument();

      chunkGates[chunk].open();

      expect(await screen.findByRole("main", { name: view })).toBeInTheDocument();
      expect(screen.queryByRole("main", { name: "Chat workspace" })).not.toBeInTheDocument();
    },
  );

  it("shows the recovery surface, not an empty region, when a view chunk fails to load", async () => {
    render(
      <AppErrorBoundary reload={() => undefined}>
        <AppShell bootstrap={bootstrap} />
      </AppErrorBoundary>,
    );
    await screen.findByRole("main", { name: "Chat workspace" });

    act(() => {
      openCanvasArtifact({
        id: "canvas-broken-chunk",
        type: "table",
        title: "Broken chunk",
        columns: ["Item"],
        rows: [["Widget"]],
        numberFormats: [null],
      });
    });
    chunkGates.canvas.fail(new Error("Failed to fetch dynamically imported module"));

    expect(await screen.findByRole("heading", { name: "xFalcon needs to recover" })).toBeInTheDocument();
    expect(screen.queryByRole("complementary", { name: "Artifact side panel" })).not.toBeInTheDocument();
  });
});
