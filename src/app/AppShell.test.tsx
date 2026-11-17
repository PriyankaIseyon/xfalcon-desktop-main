import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppBootstrap, ReportListResult, ReportView } from "../lib/generated/contracts";
import {
  applyConversationView,
  beginConversationOpen,
  applyConversationPreview,
  completeConversationOpen,
  getChatSnapshot,
  stopChatEventBridge,
  applyToolInvocationForTests,
  resetChatStore,
  setChatStoreForTests,
  setWorkspaceBootstrap,
} from "../features/chat/chatStore";
import { resetAuthSessionForTests, setAuthSession } from "../features/auth/authStore";
import { openCanvasArtifact, resetCanvasStore } from "../features/canvas/canvasStore";
import { getSignedOutNotice } from "../features/auth/signedOutNotice";
import { resetDatasetsStore } from "../features/datasets/datasetsStore";
import { getReportsSnapshot, resetReportsStore } from "../features/reports/reportsStore";
import { resetToastsForTests } from "../ui/toastStore";
import { Toaster } from "../ui/Toaster";
import { AppShell } from "./AppShell";

const invokeMock = vi.hoisted(() => vi.fn());
const listenMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  convertFileSrc: (id: string) => `http://xf-preview.localhost/${id}`,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

/** The handler the shell installed for `event`, once it has installed it. */
async function listenerFor(event: string): Promise<(message: { payload: unknown }) => void> {
  let handler: ((message: { payload: unknown }) => void) | undefined;
  await waitFor(() => {
    const call = listenMock.mock.calls.find(([name]) => name === event);
    expect(call, `nothing is listening on ${event}`).toBeDefined();
    handler = call?.[1];
  });
  return handler as (message: { payload: unknown }) => void;
}

const workspaceStatusBase = {
  version: 1,
  tenant_host: "tenant.example",
  mcp_resource_url: "https://tenant.example/mcp",
  mcp_servers: [],
  models_available: 1,
  default_model_id: "gpt-test",
  default_provider: "openai",
  setup_note: "Bundled MCP is automatic.",
} as const;

function mockNativeBoundary(chatReady: boolean) {
  invokeMock.mockImplementation(async (command: string) => {
    if (command === "workspace_status") {
      return { ...workspaceStatusBase, chat_ready: chatReady };
    }
    if (command === "chat_workspace_bootstrap") {
      return { version: 1, conversations: [], recovery_notice: null };
    }
    if (command === "chat_conversation_preview") {
      return null;
    }
    if (command === "chat_conversation_reset_active") {
      return undefined;
    }
    if (command === "project_find_by_folder") return { version: 1, projects: [] };
    if (command === "project_list") {
      return { version: 1, projects: [] };
    }
    if (command === "credential_list") {
      return [];
    }
    if (command === "mcp_server_list") {
      return { version: 1, servers: [] };
    }
    if (command === "report_list") {
      return { version: 1, reports: [] };
    }
    if (command === "dataset_list") {
      return { version: 1, datasets: [], uploads_enabled: true, truncated: false };
    }
    if (command === "skill_list") {
      return { version: 1, skills: [] };
    }
    throw new Error(`Unexpected invoke: ${command}`);
  });
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

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
  mockNativeBoundary(true);
});

describe("AppShell", () => {
  // Runs first on purpose. The first render of the lazy Datasets view suspends.
  // Once any test has rendered it, a later mount happens inside the first click,
  // before the second click can cancel it. So no earlier test may open Datasets.
  it("drops an 'Upload a dataset' request when the person leaves before Datasets opens", async () => {
    // The request asks Datasets to open its file picker once it mounts. Leaving
    // before it mounts must drop the request, or the picker would open unasked on
    // a later visit to Datasets.
    const user = userEvent.setup();
    const commandCount = (command: string) => invokeMock.mock.calls.filter(([name]) => name === command).length;
    render(<AppShell bootstrap={bootstrap} />);
    await screen.findByRole("button", { name: "Datasets" });

    fireEvent.click(screen.getByRole("button", { name: "Upload a dataset" }));
    fireEvent.click(screen.getByRole("button", { name: "Chats" }));

    await user.click(screen.getByRole("button", { name: "Datasets" }));
    await screen.findByRole("main", { name: "Datasets" });
    await waitFor(() => expect(commandCount("dataset_list")).toBe(1));
    expect(commandCount("dataset_pick")).toBe(0);
  });

  it.each(["center", "sidebar"])("reuses a shared-folder project from the %s creation flow", async (entry) => {
    const user = userEvent.setup();
    const existing = {
      version: 1,
      id: "project-existing",
      display_name: "Research",
      root_path: "C:\\Research",
      status: "trusted",
      active: false,
      trusted_at_ms: 0,
    };
    let activated = false;
    const defaultInvoke = invokeMock.getMockImplementation()!;
    invokeMock.mockImplementation(async (command: string, args: unknown) => {
      if (command === "project_pick_folder") {
        return { version: 1, grant_id: "grant-existing", display_name: "Research", display_path: existing.root_path };
      }
      if (command === "project_list") return { version: 1, projects: [{ ...existing, active: activated }] };
      if (command === "project_find_by_folder") return { version: 1, projects: [existing] };
      if (command === "project_activate") {
        activated = true;
        return { ...existing, active: true };
      }
      if (command === "project_list_directory") return { version: 1, entries: [] };
      return defaultInvoke(command, args);
    });
    render(<AppShell bootstrap={bootstrap} />);
    await user.click(screen.getByRole("button", { name: "Projects" }));
    const source = await screen.findByRole(entry === "center" ? "main" : "navigation", {
      name: entry === "center" ? "Project files" : "Workspace",
    });
    await user.click(await within(source).findByRole("button", { name: "New project" }));
    const dialog = screen.getByRole("dialog", { name: "New project" });
    await user.click(within(dialog).getByRole("button", { name: "Choose folder" }));
    await user.click(await within(dialog).findByRole("button", { name: "Open Research" }));
    expect(invokeMock).toHaveBeenCalledWith("project_activate", { projectId: existing.id });
    expect(invokeMock).not.toHaveBeenCalledWith("project_create", expect.anything());
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "New project" })).not.toBeInTheDocument());
    expect(await screen.findByRole("main", { name: "Project files" })).toBeInTheDocument();
    expect(await screen.findByText("This project folder is empty.")).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("project_list_directory", { projectId: existing.id, path: null });
  });

  it.each(["chat button", "chat menu", "project button", "project menu"])(
    "confirms deletion from the %s and preserves the chat on cancel or failure",
    async (entry) => {
      const user = userEvent.setup();
      vi.stubGlobal("__TAURI_INTERNALS__", {});
      const deletion = deferred<null>();
      let attempts = 0;
      let deleted = false;
      const conversation = {
        version: 1 as const,
        conversation_id: "conv-delete",
        title: "Quarterly results",
        updated_at: new Date().toISOString(),
        project_id: entry.startsWith("project") ? "project-1" : null,
      };
      invokeMock.mockImplementation(async (command: string) => {
        if (command === "workspace_status") return { ...workspaceStatusBase, chat_ready: true };
        if (command === "chat_workspace_bootstrap") return { version: 1, conversations: deleted ? [] : [conversation] };
        if (command === "project_list")
          return {
            version: 1,
            projects: [
              {
                version: 1,
                id: "project-1",
                display_name: "Research",
                root_path: "C:\\research",
                status: "trusted",
                active: true,
                trusted_at_ms: 0,
              },
            ],
          };
        if (command === "project_list_directory") return { version: 1, entries: [] };
        if (command === "chat_conversation_delete") {
          attempts += 1;
          if (attempts === 1) throw new Error("Unavailable");
          await deletion.promise;
          deleted = true;
          return null;
        }
        return null;
      });
      render(<AppShell bootstrap={bootstrap} />);
      await waitFor(() => expect(screen.getByRole("button", { name: "New chat" })).toBeEnabled());
      act(() => setWorkspaceBootstrap([conversation]));
      if (entry.startsWith("project")) await user.click(screen.getByRole("button", { name: "Projects" }));
      const openDelete = async () => {
        if (entry.endsWith("button")) {
          await user.click(await screen.findByRole("button", { name: "Delete Quarterly results" }));
        } else {
          const row = await screen.findByRole(entry.startsWith("project") ? "treeitem" : "option", {
            name: /Quarterly results/,
          });
          await user.pointer({ target: row, keys: "[MouseRight]" });
          await user.click(
            screen.getByRole("menuitem", { name: entry.startsWith("project") ? "Delete chat" : "Delete" }),
          );
        }
      };
      const initialConversations = getChatSnapshot().conversations;
      await openDelete();
      expect(attempts).toBe(0);
      let dialog = screen.getByRole("alertdialog", { name: "Delete chat?" });
      expect(dialog).toHaveTextContent("Quarterly results");
      await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
      expect(attempts).toBe(0);
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
      await openDelete();
      await user.keyboard("{Escape}");
      expect(attempts).toBe(0);
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
      await openDelete();
      dialog = screen.getByRole("alertdialog", { name: "Delete chat?" });
      await user.click(within(dialog).getByRole("button", { name: "Delete chat" }));
      expect(await within(dialog).findByText("The chat could not be deleted. Try again.")).toBeInTheDocument();
      expect(attempts).toBe(1);
      expect(getChatSnapshot().conversations).toEqual(initialConversations);
      await user.click(within(dialog).getByRole("button", { name: "Delete chat" }));
      expect(within(dialog).getByRole("button", { name: "Delete chat" })).toBeDisabled();
      await user.click(within(dialog).getByRole("button", { name: "Delete chat" }));
      expect(attempts).toBe(2);
      expect(invokeMock).toHaveBeenCalledWith("chat_conversation_delete", { conversationId: "conv-delete" });
      await act(async () => {
        deletion.resolve(null);
        await deletion.promise;
      });
      await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
      expect(getChatSnapshot().conversations).toEqual([]);
      expect(screen.queryByRole("button", { name: "Delete Quarterly results" })).not.toBeInTheDocument();
    },
  );

  it("starts in chat, with the other surfaces navigable but not active", () => {
    render(<AppShell bootstrap={bootstrap} />);

    expect(screen.getByRole("main", { name: "Chat workspace" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "What can I help you with?" })).toBeInTheDocument();
    expect(screen.queryByRole("complementary", { name: "Artifact canvas" })).not.toBeInTheDocument();
    // Settings is reachable from the sidebar and chat is still what the shell
    // opens on. Artifacts are not a top-level destination any more -- they
    // belong to a project, and are reached by opening one.
    expect(screen.getByRole("button", { name: "Chats" })).toHaveAttribute("aria-current", "page");
    // The group offers all three surfaces from the first frame, and marks the
    // one you are on -- the shell answers "where am I" without being asked.
    expect(screen.getByRole("button", { name: "Datasets" })).not.toHaveAttribute("aria-current");
    expect(screen.getByRole("button", { name: "Reports" })).not.toHaveAttribute("aria-current");
    expect(screen.getByRole("button", { name: "Settings" })).not.toHaveAttribute("aria-current");
    expect(screen.queryByRole("button", { name: "Browse files" })).toBeNull();
    expect(screen.queryByText("Skills")).not.toBeInTheDocument();
  });

  it.each(["windows", "macos"] as const)(
    "marks the title bar and brand as window drag regions only on macOS (%s)",
    (platform) => {
      // The macOS title bar is an overlay, so the header is the drag handle there. Elsewhere the drag
      // permission is not granted, and the attribute would send a denied call on every press.
      const { container } = render(<AppShell bootstrap={{ ...bootstrap, platform }} />);
      const header = container.querySelector("header.app-titlebar");
      const brand = container.querySelector(".app-sidebar__brand");

      if (platform === "macos") {
        expect(header).toHaveAttribute("data-tauri-drag-region", "deep");
        expect(brand).toHaveAttribute("data-tauri-drag-region", "deep");
      } else {
        expect(header).not.toHaveAttribute("data-tauri-drag-region");
        expect(brand).not.toHaveAttribute("data-tauri-drag-region");
      }
    },
  );

  it("titles every destination after itself, never after another one", async () => {
    // The titlebar name used to be a ternary chain whose else-branch was the
    // files case. Pinning each destination's name by behaviour means a new
    // destination cannot ship silently wearing the active project's name.
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "workspace_status") return { ...workspaceStatusBase, chat_ready: true };
      if (command === "chat_workspace_bootstrap") return { version: 1, conversations: [], recovery_notice: null };
      if (command === "project_list") {
        return {
          version: 1,
          projects: [
            {
              version: 1,
              id: "project-1",
              display_name: "Research",
              root_path: "C:\\work\\research",
              status: "trusted",
              active: true,
              trusted_at_ms: 0,
            },
          ],
        };
      }
      if (command === "project_list_directory") {
        return { version: 1, project_id: "project-1", path: null, entries: [] };
      }
      if (command === "credential_list") return [];
      if (command === "mcp_server_list") return { version: 1, servers: [] };
      if (command === "report_list") return { version: 1, reports: [] };
      if (command === "dataset_list") return { version: 1, datasets: [], uploads_enabled: true, truncated: false };
      if (command === "skill_list") return { version: 1, skills: [] };
      throw new Error(`Unexpected invoke: ${command}`);
    });
    const user = userEvent.setup();
    const { container } = render(<AppShell bootstrap={bootstrap} />);
    const titlebarName = () => container.querySelector(".app-titlebar strong")?.textContent;

    expect(titlebarName()).toBe("New conversation");

    await user.click(screen.getByRole("button", { name: "Search" }));
    await user.click(await screen.findByRole("option", { name: /Browse project artifacts/ }));
    await screen.findByRole("main", { name: "Project files" });
    expect(titlebarName()).toBe("Research");

    await user.click(screen.getByRole("button", { name: "Reports" }));
    await screen.findByRole("main", { name: "Reports" });
    expect(titlebarName()).toBe("Reports");

    await user.click(screen.getByRole("button", { name: "Settings" }));
    await screen.findByRole("main", { name: "Settings" });
    expect(titlebarName()).toBe("Settings");
  });

  it("asks the workspace for a dataset catalog only once someone opens Datasets", async () => {
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);

    const listed = () => invokeMock.mock.calls.filter(([command]) => command === "dataset_list").length;

    await screen.findByRole("button", { name: "Datasets" });
    expect(listed()).toBe(0);

    await user.click(screen.getByRole("button", { name: "Datasets" }));
    await waitFor(() => expect(listed()).toBe(1));
  });

  it("brings a dropped dataset to its destination, wherever the person was", async () => {
    // The backend stages a dropped file whatever is on screen. Without the
    // shell routing to the destination, that staged file would be invisible and
    // would hold a staging slot until the app exits.
    render(<AppShell bootstrap={bootstrap} />);
    const dropped = await listenerFor("xfalcon://dataset-dropped");

    expect(screen.getByRole("main", { name: "Chat workspace" })).toBeInTheDocument();
    act(() => {
      dropped({
        payload: {
          version: 1,
          staged: {
            version: 1,
            id: "staged-1",
            display_name: "sales.csv",
            size: 2048,
            format: "csv",
            worksheets: [],
          },
          error: null,
        },
      });
    });

    expect(await screen.findByRole("main", { name: "Datasets" })).toBeInTheDocument();
    // The confirmation step opened on the dropped file: the dialog names it on
    // its file card rather than in its title.
    expect(await screen.findByText("sales.csv")).toBeInTheDocument();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("makes Datasets a destination that names itself and unmounts chat", async () => {
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);

    const datasets = screen.getByRole("button", { name: "Datasets" });
    expect(datasets).not.toHaveAttribute("aria-current");

    await user.click(datasets);

    expect(await screen.findByRole("main", { name: "Datasets" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Datasets" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("button", { name: "Chats" })).not.toHaveAttribute("aria-current");
    expect(screen.queryByRole("textbox", { name: "Message xFalcon" })).not.toBeInTheDocument();
    // The lists under Chats belong to Chats. On another destination the column
    // below the group is that destination's, and it has nothing to put there.
    expect(screen.queryByText("Recents")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "New chat" })).not.toBeInTheDocument();
    expect(await screen.findByText("No datasets yet")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Chats" }));
    expect(screen.getByRole("main", { name: "Chat workspace" })).toBeInTheDocument();
  });

  it("reaches Datasets from the command palette", async () => {
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);

    await user.click(screen.getByRole("button", { name: "Search" }));
    await user.click(await screen.findByRole("option", { name: /Datasets/ }));
    expect(await screen.findByRole("main", { name: "Datasets" })).toBeInTheDocument();
  });

  it("opens the artifact side panel and the chat export from their triggers once their views load", async () => {
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);
    await screen.findByRole("main", { name: "Chat workspace" });

    act(() => {
      applyConversationView({
        version: 1,
        conversation_id: "conv-export",
        title: "Exportable thread",
        availability: "live",
        messages: [
          {
            version: 1,
            message_id: "m-export",
            role: "user",
            text: "Export this turn",
            sequence: BigInt(1),
            tool_calls: [],
            attachments: [],
          },
        ],
        active_run_id: null,
      });
    });
    await user.click(await screen.findByRole("button", { name: "Export chat" }));
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());

    act(() => {
      openCanvasArtifact({
        id: "canvas-lazy",
        type: "table",
        title: "Lazy table",
        columns: ["Item"],
        rows: [["Widget"]],
        numberFormats: [null],
      });
    });
    expect(await screen.findByRole("complementary", { name: "Artifact side panel" })).toBeInTheDocument();
    expect((await screen.findAllByText("Lazy table")).length).toBeGreaterThan(0);
  });

  it("makes Reports a destination that names itself and unmounts chat", async () => {
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);

    // Present, reachable, and not the current destination.
    const reports = screen.getByRole("button", { name: "Reports" });
    expect(reports).not.toHaveAttribute("aria-current");

    await user.click(reports);

    expect(await screen.findByRole("main", { name: "Reports" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reports" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("button", { name: "Chats" })).not.toHaveAttribute("aria-current");
    // Chat is unmounted rather than hidden: the chat column's own `display:
    // flex` would win over the `hidden` attribute.
    expect(screen.queryByRole("textbox", { name: "Message xFalcon" })).not.toBeInTheDocument();
    expect(await screen.findByText("No reports yet")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Chats" }));
    expect(screen.getByRole("main", { name: "Chat workspace" })).toBeInTheDocument();
  });

  it("does not let a palette prefetch undo a confirmed report mutation", async () => {
    const report: ReportView = {
      version: 1,
      id: "report-1",
      kind: "dashboard",
      title: "Q3 pipeline",
      description: "Bookings by region",
      created_at_ms: 1n,
      updated_at_ms: 1n,
      thumbnail_updated_at_ms: null,
      data_sources: [],
      pinned: false,
      hidden: false,
      byte_size: 100n,
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
    };
    const pin = deferred<ReportView>();
    const paletteList = deferred<ReportListResult>();
    let reportLists = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "workspace_status") return { ...workspaceStatusBase, chat_ready: true };
      if (command === "chat_workspace_bootstrap") return { version: 1, conversations: [], recovery_notice: null };
      if (command === "project_list") return { version: 1, projects: [] };
      if (command === "credential_list") return [];
      if (command === "mcp_server_list") return { version: 1, servers: [] };
      if (command === "dataset_list") return { version: 1, datasets: [], uploads_enabled: true, truncated: false };
      if (command === "skill_list") return { version: 1, skills: [] };
      if (command === "report_list") {
        reportLists += 1;
        return reportLists === 1 ? { version: 1, reports: [report] } : paletteList.promise;
      }
      if (command === "report_set_pinned") return pin.promise;
      throw new Error(`Unexpected invoke: ${command}`);
    });
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);

    await user.click(screen.getByRole("button", { name: "Reports" }));
    await user.click(await screen.findByRole("button", { name: /^Pin Q3 pipeline/ }));
    await user.click(screen.getByRole("button", { name: "Search" }));
    await waitFor(() => expect(reportLists).toBe(2));

    const pinned = { ...report, pinned: true, updated_at_ms: 2n };
    await act(async () => pin.resolve(pinned));
    await act(async () => paletteList.resolve({ version: 1, reports: [report], project_origins: [] }));

    expect(getReportsSnapshot().reports).toEqual([pinned]);
  });

  it("shows settings when asked and returns to the same chat surface afterwards", async () => {
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);

    await user.click(screen.getByRole("button", { name: "Settings" }));

    expect(await screen.findByRole("main", { name: "Settings" })).toBeInTheDocument();
    const modelsConnections = screen.getByRole("heading", { name: "Models & connections" }).closest("section");
    expect(modelsConnections).toContainElement(screen.getByRole("group", { name: "Default model" }));
    expect(modelsConnections).toContainElement(screen.getByRole("group", { name: "Private models" }));
    expect(await screen.findByRole("heading", { name: "Model providers" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "MCP servers" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Settings" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("button", { name: "Chats" })).not.toHaveAttribute("aria-current");
    expect(screen.queryByRole("textbox", { name: "Message xFalcon" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Chats" }));

    expect(screen.getByRole("main", { name: "Chat workspace" })).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Message xFalcon" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Chats" })).toHaveAttribute("aria-current", "page");
  });

  it.each(["windows", "macos"] as const)("adds a project from the empty workspace on %s", async (platform) => {
    const user = userEvent.setup();
    const path = platform === "windows" ? "C:\\Research" : "/Users/test/Research";
    let created = false;
    const newProject = {
      version: 1,
      id: "project-new",
      display_name: "Research",
      root_path: path,
      status: "trusted",
      active: true,
      trusted_at_ms: 0,
    };
    const defaultInvoke = invokeMock.getMockImplementation()!;
    invokeMock.mockImplementation(async (command: string, args: unknown) => {
      if (command === "project_pick_folder") {
        return { version: 1, grant_id: "grant-new", display_name: "Research", display_path: path };
      }
      if (command === "project_create") {
        created = true;
        return newProject;
      }
      if (command === "project_list") return { version: 1, projects: created ? [newProject] : [] };
      if (command === "project_list_directory") return { version: 1, entries: [] };
      return defaultInvoke(command, args);
    });
    render(<AppShell bootstrap={{ ...bootstrap, platform }} />);

    await user.click(screen.getByRole("button", { name: "Projects" }));

    const workspace = await screen.findByRole("main", { name: "Project files" });
    expect(await screen.findByText("No project is open")).toBeInTheDocument();
    await user.click(within(workspace).getByRole("button", { name: "New project" }));
    const dialog = screen.getByRole("dialog", { name: "New project" });
    await user.click(within(dialog).getByRole("button", { name: "Choose folder" }));
    expect(invokeMock).toHaveBeenCalledWith("project_pick_folder");
    expect(await within(dialog).findByRole("textbox", { name: "Project name" })).toHaveValue("Research");
    expect(invokeMock).not.toHaveBeenCalledWith("project_create", expect.anything());
    await user.click(within(dialog).getByRole("button", { name: "Trust & create project" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "New project" })).not.toBeInTheDocument());
    // The renderer names the folder by its grant, never by the path.
    expect(invokeMock).toHaveBeenCalledWith("project_create", { grantId: "grant-new", displayName: "Research" });
    expect(await screen.findByRole("treeitem", { name: /Research/ })).toBeInTheDocument();
    expect(screen.queryByText("No project is open")).not.toBeInTheDocument();
  });

  it("writes a file edited in the explorer, with no review step in between", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "workspace_status") return { ...workspaceStatusBase, chat_ready: true };
      if (command === "chat_workspace_bootstrap") return { version: 1, conversations: [], recovery_notice: null };
      if (command === "project_list") {
        return {
          version: 1,
          projects: [
            {
              version: 1,
              id: "project-1",
              display_name: "Research",
              root_path: "/home/user/research",
              status: "trusted",
              active: true,
              trusted_at_ms: 0,
            },
          ],
        };
      }
      if (command === "project_list_directory") {
        return {
          version: 1,
          project_id: "project-1",
          path: null,
          entries: [{ name: "notes.md", kind: "file", size_bytes: 5, is_sensitive: false }],
        };
      }
      if (command === "project_file_stat") {
        return { kind: "text", version: 1, path: "notes.md", size_bytes: 3, modified_ms: 1 };
      }
      if (command === "project_file_read_bytes") {
        return new TextEncoder().encode("old").buffer;
      }
      if (command === "project_save_file") {
        return null;
      }
      throw new Error(`Unexpected invoke: ${command}`);
    });
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);

    await user.click(screen.getByRole("button", { name: "Projects" }));
    await user.click(await screen.findByRole("treeitem", { name: "Browse files" }));
    await user.click(await screen.findByRole("treeitem", { name: /notes\.md/ }));
    await user.click(await screen.findByRole("button", { name: "Edit" }));

    const editor = screen.getByRole("textbox", { name: "File contents" });
    await user.clear(editor);
    await user.type(editor, "new");
    await user.click(screen.getByRole("button", { name: "Save" }));

    // Save writes. These are local files the operator trusted, and the person
    // is looking at their own edit -- there is nothing left to review.
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("project_save_file", {
        projectId: "project-1",
        path: "notes.md",
        contentBase64: "bmV3",
      }),
    );
    expect(await screen.findByText("Saved notes.md")).toBeInTheDocument();
  });

  it("saves a project chat's dashboard into its project and opens the written file", async () => {
    const html =
      "<!DOCTYPE html><html><head><title>Q3 revenue</title></head><body><div>Bookings by region for the third quarter, charted</div></body></html>";
    invokeMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "workspace_status") return { ...workspaceStatusBase, chat_ready: true };
      if (command === "chat_workspace_bootstrap") return { version: 1, conversations: [], recovery_notice: null };
      if (command === "project_list") {
        return {
          version: 1,
          projects: [
            {
              version: 1,
              id: "project-1",
              display_name: "Research",
              root_path: "/home/user/research",
              status: "trusted",
              active: true,
              trusted_at_ms: 0,
            },
          ],
        };
      }
      if (command === "preview_document_register") return "beef".repeat(16);
      if (command === "report_save_to_project") {
        return {
          version: 1,
          status: "saved",
          file: { version: 1, project_id: "project-1", relative_path: "reports/Q3 revenue.html" },
          limit: null,
          external_reference: false,
        };
      }
      if (command === "project_list_directory") {
        return {
          version: 1,
          project_id: "project-1",
          path: args?.path ?? null,
          entries:
            args?.path === "reports"
              ? [{ name: "Q3 revenue.html", kind: "file", size_bytes: html.length, is_sensitive: false }]
              : [{ name: "reports", kind: "directory", size_bytes: 0, is_sensitive: false }],
        };
      }
      if (command === "project_file_stat") {
        return {
          kind: "text",
          version: 1,
          path: "reports/Q3 revenue.html",
          size_bytes: new TextEncoder().encode(html).length,
          modified_ms: 1,
        };
      }
      if (command === "project_file_read_bytes") {
        return new TextEncoder().encode(html).buffer;
      }
      if (command === "project_preview_register") return "cafe".repeat(16);
      throw new Error(`Unexpected invoke: ${command}`);
    });
    const user = userEvent.setup();
    render(
      <>
        <AppShell bootstrap={bootstrap} />
        <Toaster />
      </>,
    );
    act(() => {
      setWorkspaceBootstrap([
        {
          version: 1,
          conversation_id: "conv-project",
          title: "Project thread",
          updated_at: new Date().toISOString(),
          project_id: "project-1",
        },
      ]);
      applyConversationView({
        version: 1,
        conversation_id: "conv-project",
        title: "Project thread",
        availability: "live",
        messages: [
          {
            version: 1,
            message_id: "3f2a6c48-1e5b-4d7a-9c31-0b8e5f6a2d14",
            role: "assistant",
            text: `Here it is.\n\n\`\`\`html\n${html}\n\`\`\``,
            sequence: BigInt(1),
            tool_calls: [],
            attachments: [],
          },
        ],
        active_run_id: null,
      });
    });

    await user.click(await screen.findByRole("button", { name: "Save to project: Q3 revenue (Research)" }));
    await user.click(screen.getByRole("button", { name: "Save to project" }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("report_save_to_project", {
        conversationId: "conv-project",
        messageId: "3f2a6c48-1e5b-4d7a-9c31-0b8e5f6a2d14",
        htmlIndex: 0,
        title: "Q3 revenue",
        allowDuplicate: false,
      }),
    );
    await user.click(await screen.findByRole("button", { name: "Open file" }));
    expect(await screen.findByRole("main", { name: "Project files" })).toBeInTheDocument();
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("project_preview_register", {
        projectId: "project-1",
        path: "reports/Q3 revenue.html",
        kind: "html",
      }),
    );
  });

  it("operates the sidebar and command palette from the keyboard", async () => {
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);

    // Settle first, and settle on the thing that actually moves focus. The
    // composer claims it once the workspace reports ready, on a promise of its
    // own: waiting for the empty state to paint is not the same wait, and this
    // test used to focus the toggle, lose it a tick later, and type Enter into
    // the composer.
    await screen.findByRole("heading", { name: "What can I help you with?" });
    const composer = within(screen.getByRole("form", { name: "Message composer" })).getByRole("textbox");
    await waitFor(() => expect(composer).toHaveFocus());

    const sidebarToggle = screen.getByRole("button", { name: "Hide sidebar" });
    sidebarToggle.focus();
    await user.keyboard("{Enter}");
    expect(screen.getByRole("button", { name: "Show sidebar" })).toHaveAttribute("aria-expanded", "false");
    // Collapsed is a rail on a window with room for one: still on screen, still
    // reachable, and never inert -- inert belongs to the narrow drawer, which is
    // the only state that takes the sidebar off screen entirely.
    const railed = screen.getByRole("navigation", { name: "Workspace" });
    expect(railed.closest("aside")).toHaveAttribute("data-rail", "true");
    expect(railed.closest("aside")).not.toHaveAttribute("inert");
    // By accessible name, not by printed text: the rail and the sidebar
    // print the SAME shortcut, and they used to disagree ("Ctrl K" against
    // "Ctrl+K") precisely because each spelled it itself.
    expect(screen.getAllByRole("img", { name: "Control plus K" })).toHaveLength(1);

    await user.keyboard("{Control>}k{/Control}");
    expect(screen.getByRole("dialog", { name: "Search" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "New chat" })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Search" })).not.toBeInTheDocument();
  });

  it("surfaces an offline banner when the browser reports no connectivity", () => {
    const online = Object.getOwnPropertyDescriptor(navigator, "onLine");
    Object.defineProperty(navigator, "onLine", { configurable: true, value: false });
    try {
      render(<AppShell bootstrap={bootstrap} />);
      expect(screen.getByText(/you appear to be offline/i)).toBeInTheDocument();
    } finally {
      if (online) Object.defineProperty(navigator, "onLine", online);
    }
  });

  it("announces reconnection once connectivity returns, and stays quiet on a session that starts online", () => {
    // The banner's own disappearance announces nothing -- role="status" only
    // speaks for an addition or update, not a removal -- so a screen reader
    // user (or anyone not looking at that exact spot) had no signal that
    // connectivity was back at all.
    try {
      // `Toaster` is a sibling of `AppShell` under `App`, not a child of it --
      // rendered alongside here to match, since the reconnect toast is
      // otherwise pushed to a store nothing in this tree is mounted to show.
      render(
        <>
          <AppShell bootstrap={bootstrap} />
          <Toaster />
        </>,
      );
      expect(screen.queryByText("You're back online.")).not.toBeInTheDocument();

      act(() => window.dispatchEvent(new Event("offline")));
      expect(screen.getByText(/you appear to be offline/i)).toBeInTheDocument();
      expect(screen.queryByText("You're back online.")).not.toBeInTheDocument();

      act(() => window.dispatchEvent(new Event("online")));
      expect(screen.queryByText(/you appear to be offline/i)).not.toBeInTheDocument();
      expect(screen.getByText("You're back online.")).toBeInTheDocument();
    } finally {
      // The toast store is module state outside React's tree, so `cleanup()`
      // does not clear it -- left alone, the real 3s auto-dismiss timer this
      // toast carries would fire mid-way through a later test in this file.
      resetToastsForTests();
    }
  });

  it("shows the agent-steps card only once a tool has actually run", async () => {
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);

    // No tool has run: no disclosure furniture on the welcome surface.
    expect(screen.queryByRole("button", { name: /tool call/i })).not.toBeInTheDocument();

    act(() => {
      // Tool events are admitted only for the run bound to the view, so the
      // conversation with its active run has to be on screen first.
      applyConversationView({
        version: 1,
        conversation_id: "conv-activity",
        title: "Tool thread",
        availability: "live",
        messages: [
          {
            version: 1,
            message_id: "m-1",
            role: "user",
            text: "read the file",
            sequence: BigInt(1),
            tool_calls: [],
            attachments: [],
          },
        ],
        active_run_id: "run-1",
      });
      applyToolInvocationForTests({
        type: "requested",
        version: 1,
        invocation_id: "inv-1",
        run_id: "run-1",
        tool_id: "project.read_file",
      });
    });

    // The projection has identified an active run even though its first
    // lifecycle state has not arrived yet. That interval is still working, so
    // the same pill stays active instead of disappearing until the
    // later state event. It is closed until the reader opens it.
    const disclosure = screen.getByRole("button", { name: /Thinking/i });
    expect(disclosure).toHaveAttribute("aria-expanded", "false");

    await user.click(disclosure);
    expect(disclosure).toHaveAttribute("aria-expanded", "true");
  });

  it("disables the composer when workspace chat is not ready", async () => {
    mockNativeBoundary(false);
    render(<AppShell bootstrap={bootstrap} />);

    await waitFor(() => {
      expect(screen.getByRole("textbox", { name: "Message xFalcon" })).toBeDisabled();
    });
    expect(screen.getByRole("textbox", { name: "Message xFalcon" })).toHaveAttribute(
      "placeholder",
      "\u00a0\u00a0Chat is not connected yet",
    );
  });

  it("disables conversation recents when workspace chat is not ready", async () => {
    mockNativeBoundary(false);
    render(<AppShell bootstrap={bootstrap} />);
    await waitFor(() => {
      expect(screen.getByRole("textbox", { name: "Message xFalcon" })).toBeDisabled();
    });

    setWorkspaceBootstrap([
      {
        version: 1,
        conversation_id: "conv-sidebar",
        title: "Sidebar cached thread",
        updated_at: new Date().toISOString(),
      },
    ]);

    const recent = await screen.findByRole("option", { name: "Sidebar cached thread" });
    expect(recent).toBeDisabled();
  });

  it("keeps ordinary recents separate from the chats that belong to a project", async () => {
    const user = userEvent.setup();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "workspace_status") return { ...workspaceStatusBase, chat_ready: true };
      if (command === "chat_workspace_bootstrap") return { version: 1, conversations: [], recovery_notice: null };
      if (command === "project_list") {
        return {
          version: 1,
          projects: [
            {
              version: 1,
              id: "project-1",
              display_name: "Research",
              root_path: "C:\\work\\research",
              status: "trusted",
              active: true,
              trusted_at_ms: 0,
            },
          ],
        };
      }
      throw new Error(`Unexpected invoke: ${command}`);
    });
    render(<AppShell bootstrap={bootstrap} />);
    act(() => {
      setWorkspaceBootstrap([
        {
          version: 1,
          conversation_id: "conv-recent",
          title: "Quarterly results",
          updated_at: new Date().toISOString(),
        },
        {
          version: 1,
          conversation_id: "conv-project",
          title: "Refactor notes",
          updated_at: new Date().toISOString(),
          project_id: "project-1",
        },
      ]);
    });

    const recents = screen.getByText("Today").closest(".app-sidebar__section");
    expect(recents).not.toBeNull();
    expect(within(recents as HTMLElement).getByRole("option", { name: "Quarterly results" })).toBeInTheDocument();
    expect(within(recents as HTMLElement).queryByRole("option", { name: "Refactor notes" })).not.toBeInTheDocument();

    // A project's chats are rows of its own tree, on the destination that owns
    // that tree -- they have their own files nested under them, which a flat
    // list of options has nowhere to put.
    await user.click(screen.getByRole("button", { name: "Projects" }));
    expect(await screen.findByRole("treeitem", { name: /Refactor notes/ })).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: "Quarterly results" })).not.toBeInTheDocument();
  });

  it("renders sidebar hierarchy without legacy Chat or project rails", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "workspace_status") return { ...workspaceStatusBase, chat_ready: true };
      if (command === "chat_workspace_bootstrap") return { version: 1, conversations: [], recovery_notice: null };
      if (command === "project_list") {
        return {
          version: 1,
          projects: [
            {
              version: 1,
              id: "project-1",
              display_name: "Research",
              root_path: "C:\\work\\research",
              status: "trusted",
              active: true,
              trusted_at_ms: 0,
            },
          ],
        };
      }
      throw new Error(`Unexpected invoke: ${command}`);
    });
    const user = userEvent.setup();
    const { container } = render(<AppShell bootstrap={bootstrap} />);
    act(() => {
      setWorkspaceBootstrap([
        {
          version: 1,
          conversation_id: "conv-project",
          title: "Project thread",
          updated_at: new Date().toISOString(),
          project_id: "project-1",
        },
      ]);
    });

    await user.click(screen.getByRole("button", { name: "Projects" }));
    expect(await screen.findByRole("treeitem", { name: /Project thread/ })).toBeInTheDocument();
    expect(container.querySelector(".tree-branch-container")).toBeNull();
    expect(container.querySelector(".sidebar-rail")).toBeNull();
  });

  it("takes the recents list away with Chats when Settings becomes current", async () => {
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);
    act(() => {
      setWorkspaceBootstrap([
        {
          version: 1,
          conversation_id: "conv-flat",
          title: "Flat thread",
          updated_at: new Date().toISOString(),
        },
      ]);
      applyConversationView({
        version: 1,
        conversation_id: "conv-flat",
        title: "Flat thread",
        availability: "live",
        messages: [],
        active_run_id: null,
      });
    });

    expect(await screen.findByRole("option", { name: "Flat thread" })).toHaveAttribute("aria-selected", "true");
    await user.click(screen.getByRole("button", { name: "Settings" }));

    // A chat painted as selected while Settings is on screen would disagree
    // with the destination group. The list leaves with its destination, which
    // settles the question rather than restating it on every row.
    expect(screen.getByRole("button", { name: "Settings" })).toHaveAttribute("aria-current", "page");
    expect(screen.queryByRole("option", { name: "Flat thread" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Chats" }));
    expect(await screen.findByRole("option", { name: "Flat thread" })).toHaveAttribute("aria-selected", "true");
  });

  it("takes the project tree away with Projects when Reports becomes current", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "workspace_status") return { ...workspaceStatusBase, chat_ready: true };
      if (command === "chat_workspace_bootstrap") return { version: 1, conversations: [], recovery_notice: null };
      if (command === "project_list") {
        return {
          version: 1,
          projects: [
            {
              version: 1,
              id: "project-1",
              display_name: "Research",
              root_path: "C:\\work\\research",
              status: "trusted",
              active: true,
              trusted_at_ms: 0,
            },
          ],
        };
      }
      if (command === "report_list") return { version: 1, reports: [] };
      if (command === "dataset_list") return { version: 1, datasets: [], uploads_enabled: true, truncated: false };
      if (command === "skill_list") return { version: 1, skills: [] };
      throw new Error(`Unexpected invoke: ${command}`);
    });
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);
    act(() => {
      setWorkspaceBootstrap([
        {
          version: 1,
          conversation_id: "conv-project",
          title: "Project thread",
          updated_at: new Date().toISOString(),
          project_id: "project-1",
        },
      ]);
      applyConversationView({
        version: 1,
        conversation_id: "conv-project",
        title: "Project thread",
        availability: "live",
        messages: [],
        active_run_id: null,
      });
    });

    await user.click(screen.getByRole("button", { name: "Projects" }));
    expect(await screen.findByRole("treeitem", { name: "Research" })).toBeInTheDocument();
    expect(await screen.findByRole("main", { name: "Project files" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Reports" }));
    expect(screen.getByRole("button", { name: "Reports" })).toHaveAttribute("aria-current", "page");
    // The tree leaves with the view, and the view waits for the Reports chunk.
    expect(await screen.findByRole("main", { name: "Reports" })).toBeInTheDocument();
    expect(screen.queryByRole("treeitem", { name: "Research" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Projects" }));
    expect(await screen.findByRole("treeitem", { name: "Research" })).toBeInTheDocument();
  });

  /*
   * The bug, at the shell level: pressing a project's name started a chat in it
   * and left for Chats, so the conversation you had open was replaced by an
   * empty one and the column you were reading was swapped out.
   */
  it("keeps you on Projects when you press a project, and keeps the chat you had open", async () => {
    const base = invokeMock.getMockImplementation();
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      if (command === "project_list") {
        return {
          version: 1,
          projects: [
            {
              version: 1,
              id: "project-1",
              display_name: "Research",
              root_path: "C:\\work\\research",
              status: "trusted",
              active: true,
              trusted_at_ms: 0,
            },
          ],
        };
      }
      if (command === "project_list_directory") {
        return { version: 1, project_id: "project-1", path: null, entries: [] };
      }
      return base?.(command, args);
    });
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);
    act(() => {
      setWorkspaceBootstrap([
        {
          version: 1,
          conversation_id: "conv-project",
          title: "Project thread",
          updated_at: new Date().toISOString(),
          project_id: "project-1",
        },
      ]);
      applyConversationView({
        version: 1,
        conversation_id: "conv-project",
        title: "Project thread",
        availability: "live",
        messages: [],
        active_run_id: null,
      });
    });

    await user.click(screen.getByRole("button", { name: "Projects" }));
    const projectRow = await screen.findByRole("treeitem", { name: "Research" });
    expect(projectRow).toHaveAttribute("aria-expanded", "true");

    await user.click(projectRow);

    // It folded, and that is all it did: same destination, same conversation.
    expect(screen.getByRole("treeitem", { name: "Research" })).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByRole("button", { name: "Projects" })).toHaveAttribute("aria-current", "page");
    expect(await screen.findByRole("main", { name: "Project files" })).toBeInTheDocument();
  });

  /*
   * The other half of the same complaint: a project's chat was reachable only
   * by leaving Projects for Chats, so the row that looked like navigation
   * behaved like an exit. A project holds files and conversations; the
   * destination shows whichever one you picked.
   */
  it("opens a project's chat beside its own tree, without leaving Projects", async () => {
    const base = invokeMock.getMockImplementation();
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      if (command === "project_list") {
        return {
          version: 1,
          projects: [
            {
              version: 1,
              id: "project-1",
              display_name: "Research",
              root_path: "C:\\work\\research",
              status: "trusted",
              active: true,
              trusted_at_ms: 0,
            },
          ],
        };
      }
      if (command === "project_list_directory") {
        return { version: 1, project_id: "project-1", path: null, entries: [] };
      }
      return base?.(command, args);
    });
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);
    act(() => {
      setWorkspaceBootstrap([
        {
          version: 1,
          conversation_id: "conv-project",
          title: "Project thread",
          updated_at: new Date().toISOString(),
          project_id: "project-1",
        },
      ]);
    });

    await user.click(screen.getByRole("button", { name: "Projects" }));
    await user.click(await screen.findByRole("treeitem", { name: /Project thread/ }));

    // The chat is on screen, the tree is still beside it, and the group still
    // says where you are.
    expect(await screen.findByRole("main", { name: "Project chat" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Projects" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("button", { name: "Chats" })).not.toHaveAttribute("aria-current");
    // Which row reads as the open one is the tree's own rule, pinned in
    // ProjectNavList's tests -- what matters here is that the tree is still on
    // screen to carry the mark at all.
    expect(screen.getByRole("tree", { name: "Projects" })).toBeInTheDocument();

    // Pressing the destination itself comes back to its home. Otherwise a
    // destination would mean two different things depending on what you last
    // pressed inside it.
    await user.click(screen.getByRole("button", { name: "Projects" }));
    expect(await screen.findByRole("main", { name: "Project files" })).toBeInTheDocument();
  });

  /*
   * Starting a chat in a project had no affordance on screen -- only a context
   * menu item, which is a place you look once you already believe the action
   * is there. It is a button on the project's own row, and it stays put: a new
   * project chat sent to Chats would be on screen and in no list at all, since
   * Chats lists the loose conversations only.
   */
  it("starts a new chat in a project from its own row, and shows it there", async () => {
    const base = invokeMock.getMockImplementation();
    const project = {
      version: 1,
      id: "project-1",
      display_name: "Research",
      root_path: "C:\\work\\research",
      status: "trusted",
      active: true,
      trusted_at_ms: 0,
    };
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      if (command === "project_list") return { version: 1, projects: [project] };
      if (command === "project_activate") return project;
      if (command === "project_list_directory") {
        return { version: 1, project_id: "project-1", path: null, entries: [] };
      }
      return base?.(command, args);
    });
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);
    act(() => {
      setWorkspaceBootstrap([
        {
          version: 1,
          conversation_id: "conv-project",
          title: "Project thread",
          updated_at: new Date().toISOString(),
          project_id: "project-1",
        },
      ]);
    });

    await user.click(screen.getByRole("button", { name: "Projects" }));
    // Open one of the project's existing chats first, so the empty one that
    // replaces it is visibly a different conversation.
    await user.click(await screen.findByRole("treeitem", { name: /Project thread/ }));
    expect(await screen.findByRole("main", { name: "Project chat" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "New chat in Research" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("project_activate", { projectId: "project-1" });
    });
    // Same destination, tree still beside it, and the titlebar names the new
    // conversation rather than the project the subtitle already names.
    expect(await screen.findByRole("main", { name: "Project chat" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Projects" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("tree", { name: "Projects" })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText("New conversation")).toBeInTheDocument());
  });

  it("revokes project trust through the confirmed sidebar flow and removes the stale row", async () => {
    const base = invokeMock.getMockImplementation();
    let revoked = false;
    const project = {
      version: 1,
      id: "project-1",
      display_name: "Research",
      root_path: "C:\\work\\research",
      status: "trusted",
      active: true,
      trusted_at_ms: 0,
    };
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      if (command === "project_list") return { version: 1, projects: revoked ? [] : [project] };
      if (command === "project_list_directory") {
        return { version: 1, project_id: "project-1", path: null, entries: [] };
      }
      if (command === "project_revoke") {
        revoked = true;
        return undefined;
      }
      return base?.(command, args);
    });
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);
    await user.click(screen.getByRole("button", { name: "Projects" }));
    await user.click(await screen.findByRole("button", { name: "Remove Research" }));

    await user.click(screen.getByRole("button", { name: "Remove project" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("project_revoke", { projectId: "project-1" }));
    await waitFor(() => expect(screen.queryByRole("treeitem", { name: "Research" })).not.toBeInTheDocument());
    expect(screen.getByText("No projects yet.")).toBeInTheDocument();
  });

  it.each([false, true])("reconciles deleted chats when cleanup partially fails: %s", async (partialFailure) => {
    vi.stubGlobal("__TAURI_INTERNALS__", {});
    setAuthSession(bootstrap.session);
    const base = invokeMock.getMockImplementation();
    let deleted = false;
    const project = {
      version: 1,
      id: "project-1",
      display_name: "Research",
      root_path: "/Users/walter/Research",
      status: "trusted",
      active: true,
      trusted_at_ms: 0,
    };
    const conversation = {
      version: 1 as const,
      conversation_id: "conv-project",
      title: "Project thread",
      updated_at: "2026-09-08T00:00:00Z",
      project_id: "project-1",
    };
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      if (command === "project_list") {
        if (deleted)
          projection?.({
            payload: {
              type: "conversation_removed",
              version: 1,
              account_id: "tenant-test:user-test",
              conversation_id: "conv-project",
            },
          });
        if (deleted && partialFailure) throw new Error("Remaining project cleanup failed");
        return { version: 1, projects: deleted ? [] : [project] };
      }
      if (command === "chat_conversation_open") {
        return {
          version: 1,
          conversation_id: "conv-project",
          title: "Project thread",
          availability: "live",
          messages: [],
          active_run_id: null,
        };
      }
      if (command === "chat_workspace_bootstrap") {
        return { version: 1, conversations: deleted ? [] : [conversation], recovery_notice: null };
      }
      if (command === "project_list_directory") return { version: 1, project_id: "project-1", path: null, entries: [] };
      return base?.(command, args);
    });
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);
    await waitFor(() => expect(screen.getByRole("button", { name: "New chat" })).toBeEnabled());
    act(() => setWorkspaceBootstrap([conversation]));
    await user.click(screen.getByRole("button", { name: "Projects" }));
    await user.click(await screen.findByRole("treeitem", { name: /Project thread/ }));
    await waitFor(() => expect(getChatSnapshot().conversationId).toBe("conv-project"));
    const projection = await listenerFor("xfalcon://chat-projection-event");
    deleted = true;
    act(() => window.dispatchEvent(new Event("focus")));
    if (partialFailure) {
      expect(await screen.findByText("Remaining project cleanup failed")).toBeInTheDocument();
      expect(screen.getByRole("treeitem", { name: "Research" })).toBeInTheDocument();
    } else {
      await waitFor(() => expect(screen.getByText("No projects yet.")).toBeInTheDocument());
    }
    await waitFor(() => expect(getChatSnapshot().conversations).toEqual([]));
    expect(getChatSnapshot().conversationId).toBeNull();
    resetAuthSessionForTests();
  });

  it("keeps a project visible and reports a failed deletion reconciliation", async () => {
    const base = invokeMock.getMockImplementation();
    let failed = false;
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      if (command === "project_list") {
        if (failed) throw new Error("Project cleanup failed: connection unavailable");
        return {
          version: 1,
          projects: [
            {
              version: 1,
              id: "project-1",
              display_name: "Research",
              root_path: "/Users/walter/Research",
              status: "trusted",
              active: false,
              trusted_at_ms: 0,
            },
          ],
        };
      }
      return base?.(command, args);
    });
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);
    await user.click(screen.getByRole("button", { name: "Projects" }));
    await screen.findByRole("treeitem", { name: "Research" });
    failed = true;
    act(() => window.dispatchEvent(new Event("focus")));
    expect(await screen.findByText("Project cleanup failed: connection unavailable")).toBeInTheDocument();
    expect(screen.getByRole("treeitem", { name: "Research" })).toBeInTheDocument();
  });

  // The complaint this answers: a conversation reading as open while another
  // destination is on screen. Each list leaves with the destination that owns
  // it, and only one list is ever in the column.
  it("gives Chats and Projects one column each, never both at once", async () => {
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);
    await screen.findByRole("heading", { name: "What can I help you with?" });

    expect(screen.getByRole("button", { name: "New chat" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "New project" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Projects" }));

    expect(screen.getByRole("button", { name: "Projects" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("button", { name: "Chats" })).not.toHaveAttribute("aria-current");
    expect(
      await within(screen.getByRole("navigation", { name: "Workspace" })).findByRole("button", { name: "New project" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "New chat" })).not.toBeInTheDocument();
    expect(screen.queryByText("Recents")).not.toBeInTheDocument();
    expect(await screen.findByRole("main", { name: "Project files" })).toBeInTheDocument();
  });

  it("anchors the new-chat action above the list that grows", async () => {
    render(<AppShell bootstrap={bootstrap} />);
    await screen.findByRole("heading", { name: "What can I help you with?" });

    const newChat = screen.getByRole("button", { name: "New chat" });
    const recents = screen.getByText("Recents");

    expect(newChat.compareDocumentPosition(recents) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Search chats/ })).not.toBeInTheDocument();
  });

  it("withholds every new-chat entry point until chat setup is ready", async () => {
    mockNativeBoundary(false);
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);
    await screen.findByRole("heading", { name: "What can I help you with?" });

    expect(screen.getByRole("button", { name: "New chat" })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "Hide sidebar" }));
    expect(screen.getByRole("button", { name: "New chat" })).toBeDisabled();
  });

  it("keeps growing navigation in the flexible region and pins utilities at the bottom", async () => {
    render(<AppShell bootstrap={bootstrap} />);
    await screen.findByRole("heading", { name: "What can I help you with?" });

    const explore = document.querySelector(".app-sidebar__explore");
    const utilities = document.querySelector(".app-sidebar__utilities");
    expect(explore).not.toBeNull();
    expect(utilities).not.toBeNull();
    expect(within(explore as HTMLElement).getByText("Recents")).toBeInTheDocument();
    // Every destination lives in the group above the lists -- not in the region
    // that grows with them, and not in the utilities.
    const destinations = document.querySelector(".app-sidebar__destinations");
    for (const name of ["Chats", "Projects", "Datasets", "Reports"]) {
      expect(within(destinations as HTMLElement).getByRole("button", { name })).toBeInTheDocument();
      expect(within(explore as HTMLElement).queryByRole("button", { name })).not.toBeInTheDocument();
      expect(within(utilities as HTMLElement).queryByRole("button", { name })).not.toBeInTheDocument();
    }
    expect(within(utilities as HTMLElement).getByRole("button", { name: "Settings" })).toBeInTheDocument();
    expect(within(utilities as HTMLElement).getByRole("button", { name: "Account and app menu" })).toBeInTheDocument();
  });

  it("offers one sidebar hide control instead of duplicating it in the rail", async () => {
    render(<AppShell bootstrap={bootstrap} />);
    await screen.findByRole("heading", { name: "What can I help you with?" });

    expect(screen.getAllByRole("button", { name: "Hide sidebar" })).toHaveLength(1);
    expect(screen.queryByRole("button", { name: "Collapse sidebar" })).not.toBeInTheDocument();
  });

  it("draws a destination as a destination and a section title as a title", async () => {
    render(<AppShell bootstrap={bootstrap} />);
    await screen.findByRole("heading", { name: "What can I help you with?" });

    // Every destination in the group, one appearance between them.
    for (const name of ["Chats", "Projects", "Datasets", "Reports"]) {
      const destination = screen.getByRole("button", { name });
      expect(destination).toHaveClass("nav-destination");
      expect(destination.closest(".app-sidebar__destinations")).not.toBeNull();
    }
    // Settings is a destination too, but it sits with the account rather than in
    // the group -- so it keeps the row appearance of the column it sits in.
    expect(screen.getByRole("button", { name: "Settings" })).toHaveClass("nav-item--title");

    const child = screen.getByRole("button", { name: "New chat" });
    expect(child).toHaveClass("nav-item--child");
    expect(child).not.toHaveClass("nav-item--title");
    expect(screen.queryByRole("button", { name: /Search chats/ })).not.toBeInTheDocument();
    expect(child.closest(".app-sidebar__section--chat")).not.toBeNull();
  });

  it("keeps every destination one press away while the sidebar is a rail", async () => {
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);
    await screen.findByRole("heading", { name: "What can I help you with?" });

    await user.click(screen.getByRole("button", { name: "Hide sidebar" }));

    // Collapsing used to cost the whole of the sidebar's navigation. The rail
    // keeps the destinations, marks the one you are on, and drops only the
    // lists -- which is the width it was collapsed to reclaim.
    const rail = screen.getByRole("navigation", { name: "Workspace" });
    for (const name of ["Chats", "Projects", "Datasets", "Reports", "Settings"]) {
      expect(within(rail).getByRole("button", { name })).toBeInTheDocument();
    }
    expect(screen.getByRole("button", { name: "Chats" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("button", { name: "New chat" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Account and app menu" })).toBeInTheDocument();
    expect(screen.queryByText("Recents")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Datasets" }));

    expect(await screen.findByRole("main", { name: "Datasets" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Datasets" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("button", { name: "Chats" })).not.toHaveAttribute("aria-current");
    // New chat is a chat action, and chat is not the surface any more.
    expect(screen.queryByRole("button", { name: "New chat" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Show sidebar" }));
    expect(screen.getByRole("button", { name: "Datasets" })).toHaveAttribute("aria-current", "page");
    expect(await screen.findByRole("main", { name: "Datasets" })).toBeInTheDocument();
  });

  it("still opens the account menu when the sidebar is a rail", async () => {
    // Sign-out lives behind this trigger and nowhere else, so a rail that draws
    // the avatar without opening the menu would strand the session. `Menu`
    // clones its trigger to inject the click handler, which is why the rail's
    // trigger has to be a button and not a component wrapping one.
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);
    await screen.findByRole("heading", { name: "What can I help you with?" });

    await user.click(screen.getByRole("button", { name: "Hide sidebar" }));
    await user.click(screen.getByRole("button", { name: "Account and app menu" }));

    expect(screen.getByRole("menu")).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Sign out" })).toBeInTheDocument();
  });

  it("leaves project context from the composer and starts a normal chat", async () => {
    let active = true;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "workspace_status") return { ...workspaceStatusBase, chat_ready: true };
      if (command === "chat_workspace_bootstrap") return { version: 1, conversations: [], recovery_notice: null };
      if (command === "project_deactivate") {
        active = false;
        return undefined;
      }
      if (command === "project_list") {
        return {
          version: 1,
          projects: [
            {
              version: 1,
              id: "project-1",
              display_name: "Research",
              root_path: "C:\\work\\research",
              status: "trusted",
              active,
              trusted_at_ms: 0,
            },
          ],
        };
      }
      throw new Error(`Unexpected invoke: ${command}`);
    });
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);

    await user.click(await screen.findByRole("button", { name: "Remove Research project context" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("project_deactivate"));
    await waitFor(() =>
      expect(within(screen.getByRole("form", { name: "Message composer" })).queryByText("Research")).toBeNull(),
    );
    expect(screen.getByRole("heading", { name: "What can I help you with?" })).toBeInTheDocument();
  });

  it("does not call a cached preview offline while authoritative synchronization is pending", async () => {
    render(<AppShell bootstrap={bootstrap} />);
    await screen.findByRole("heading", { name: "What can I help you with?" });
    const cached = {
      version: 1 as const,
      conversation_id: "preview-sync",
      title: "Cached conversation",
      availability: "stale" as const,
      messages: [
        {
          version: 1 as const,
          message_id: "cached",
          role: "user" as const,
          text: "Saved message",
          sequence: 1n,
          tool_calls: [],
          attachments: [],
        },
      ],
      active_run_id: null,
    };
    act(() => {
      beginConversationOpen(cached.conversation_id);
      applyConversationPreview(cached, cached.conversation_id);
    });
    expect(screen.queryByText("Syncing conversation…")).not.toBeInTheDocument();
    expect(screen.getByText("Saved message")).toBeInTheDocument();
    expect(screen.queryByText("Offline copy — reconnect to sync")).not.toBeInTheDocument();
    act(() => completeConversationOpen(cached, cached.conversation_id));
    expect(await screen.findByText("Offline copy — reconnect to sync")).toBeInTheDocument();
    expect(screen.queryByText("Syncing conversation…")).not.toBeInTheDocument();
    act(() => applyConversationView({ ...cached, availability: "live" }));
    expect(screen.queryByText("Offline copy — reconnect to sync")).not.toBeInTheDocument();
  });

  it("surfaces stale availability on the active conversation", async () => {
    render(<AppShell bootstrap={bootstrap} />);
    await screen.findByRole("heading", { name: "What can I help you with?" });

    applyConversationView({
      version: 1,
      conversation_id: "conv-stale-unit",
      title: "Cached thread",
      availability: "stale",
      messages: [
        {
          version: 1,
          message_id: "m-1",
          role: "user",
          text: "Still here offline",
          sequence: BigInt(1),
          tool_calls: [],
          attachments: [],
        },
      ],
      active_run_id: null,
    });

    expect(await screen.findByText("Offline copy — reconnect to sync")).toBeInTheDocument();
    expect(screen.getByText("Still here offline")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Message xFalcon" })).toBeEnabled();
  });

  it("gives the arrival skeleton the whole window instead of the sidebar track", async () => {
    // `.app-shell` is a three-track grid, so a skeleton rendered *inside* it
    // lands in the 17rem sidebar track: ghost sidebar at full width, label
    // wrapped to three lines, ghost thread collapsed to nothing. The skeleton
    // has to BE the shell root, and this asserts no `.app-shell` is around it.
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "workspace_status") return { ...workspaceStatusBase, chat_ready: true };
      // A bootstrap that never lands holds the skeleton on screen.
      if (command === "chat_workspace_bootstrap") return new Promise(() => {});
      if (command === "project_list") return { version: 1, projects: [] };
      if (command === "credential_list") return [];
      if (command === "mcp_server_list") return { version: 1, servers: [] };
      throw new Error(`Unexpected invoke: ${command}`);
    });
    // The skeleton needs a signed-in identity and a live shell: `identity`
    // comes from the auth store, not the bootstrap prop.
    const shellGlobal = globalThis as { __TAURI_INTERNALS__?: unknown };
    shellGlobal.__TAURI_INTERNALS__ = {};
    setAuthSession(bootstrap.session);

    try {
      const { container } = render(<AppShell bootstrap={bootstrap} />);

      const skeleton = (await screen.findByText("Loading workspace…")).closest(".workspace-skeleton");
      expect(skeleton).toHaveAttribute("role", "status");
      expect(skeleton?.closest(".app-shell")).toBeNull();
      expect(container.querySelector(".app-shell")).toBeNull();
    } finally {
      delete shellGlobal.__TAURI_INTERNALS__;
      resetAuthSessionForTests();
    }
  });

  it("highlights and titles a selected chat before preview or server open resolves", async () => {
    mockNativeBoundary(true);
    const base = invokeMock.getMockImplementation();
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      if (command === "chat_conversation_preview" || command === "chat_conversation_open") {
        return new Promise(() => {});
      }
      return base?.(command, args);
    });
    const user = userEvent.setup();
    render(<AppShell bootstrap={bootstrap} />);
    await screen.findByRole("main", { name: "Chat workspace" });
    act(() => {
      setWorkspaceBootstrap([
        {
          version: 1,
          conversation_id: "conv-slow",
          title: "Slow target",
          updated_at: "2026-09-01T00:00:00Z",
        },
      ]);
    });

    const row = screen.getByRole("option", { name: "Slow target" });
    await user.click(row);

    expect(row).toHaveAttribute("aria-selected", "true");
    expect(screen.getAllByText("Slow target").length).toBeGreaterThan(1);
    expect(screen.getByRole("status", { name: "Loading conversation" })).toBeInTheDocument();
  });

  it("reveals the shell at the five-second workspace-arrival cap", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "workspace_status") return { ...workspaceStatusBase, chat_ready: true };
      if (command === "chat_workspace_bootstrap") return new Promise(() => {});
      if (command === "project_list") return { version: 1, projects: [] };
      if (command === "credential_list") return [];
      if (command === "mcp_server_list") return { version: 1, servers: [] };
      throw new Error(`Unexpected invoke: ${command}`);
    });
    const shellGlobal = globalThis as { __TAURI_INTERNALS__?: unknown };
    shellGlobal.__TAURI_INTERNALS__ = {};
    setAuthSession(bootstrap.session);
    vi.useFakeTimers();

    try {
      render(<AppShell bootstrap={bootstrap} />);
      await act(async () => {
        await Promise.resolve();
        await Promise.resolve();
      });
      expect(screen.getByText("Loading workspace…").closest('[role="status"]')).not.toBeNull();

      await act(async () => vi.advanceTimersByTime(4_999));
      expect(screen.getByText("Loading workspace…").closest('[role="status"]')).not.toBeNull();

      await act(async () => vi.advanceTimersByTime(1));
      expect(screen.queryByText("Loading workspace…")).not.toBeInTheDocument();
      expect(screen.getByRole("main", { name: "Chat workspace" })).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
      delete shellGlobal.__TAURI_INTERNALS__;
      resetAuthSessionForTests();
    }
  });

  it("holds the arrival skeleton while the workspace status call is still in flight, instead of flashing the bare shell", async () => {
    // Regression: `chatReady` reads `false` for the render or two before
    // `workspace_status` answers, same as it does for a tenant that is
    // genuinely not chat-ready. Gating the skeleton on `chatReady` alone
    // could not tell those apart, so sign-in landed on an empty, unstyled
    // shell for that gap before the skeleton cut in. A `workspace_status`
    // that never resolves pins the shell in that gap indefinitely, so if the
    // real shell leaks through here, it leaks through on every sign-in.
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "workspace_status") return new Promise(() => {});
      if (command === "chat_workspace_bootstrap") return new Promise(() => {});
      if (command === "project_list") return { version: 1, projects: [] };
      if (command === "credential_list") return [];
      if (command === "mcp_server_list") return { version: 1, servers: [] };
      throw new Error(`Unexpected invoke: ${command}`);
    });
    const shellGlobal = globalThis as { __TAURI_INTERNALS__?: unknown };
    shellGlobal.__TAURI_INTERNALS__ = {};
    setAuthSession(bootstrap.session);

    try {
      const { container } = render(<AppShell bootstrap={bootstrap} />);

      const skeleton = (await screen.findByText("Loading workspace…")).closest(".workspace-skeleton");
      expect(skeleton).toHaveAttribute("role", "status");
      expect(container.querySelector(".app-shell")).toBeNull();
      expect(screen.queryByRole("main", { name: "Chat workspace" })).not.toBeInTheDocument();
      // Both effects mount together. Chat bootstrap must not wait for the
      // unresolved workspace status call to announce `chat_ready`.
      expect(invokeMock).toHaveBeenCalledWith("workspace_status");
      expect(invokeMock).toHaveBeenCalledWith("chat_workspace_bootstrap");
    } finally {
      delete shellGlobal.__TAURI_INTERNALS__;
      resetAuthSessionForTests();
    }
  });

  it("keeps chat connected when its bootstrap succeeds but the parallel status summary fails", async () => {
    mockNativeBoundary(true);
    const base = invokeMock.getMockImplementation();
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      if (command === "workspace_status") throw new Error("tenant workspace is unavailable");
      return base?.(command, args);
    });
    const shellGlobal = globalThis as { __TAURI_INTERNALS__?: unknown };
    shellGlobal.__TAURI_INTERNALS__ = {};
    setAuthSession(bootstrap.session);

    try {
      render(<AppShell bootstrap={bootstrap} />);

      const composer = await screen.findByRole("textbox", { name: "Message xFalcon" });
      await waitFor(() => expect(composer).toBeEnabled());
      expect(composer).not.toHaveAttribute("placeholder", "Chat is not connected yet");
      expect(invokeMock).toHaveBeenCalledWith("workspace_status");
      expect(invokeMock).toHaveBeenCalledWith("chat_workspace_bootstrap");
    } finally {
      delete shellGlobal.__TAURI_INTERNALS__;
      resetAuthSessionForTests();
    }
  });

  it("falls through to the real shell, not the skeleton, once workspace status settles on a not-ready tenant", async () => {
    // The other half of the same fix: `statusPending` must not swallow a
    // tenant that has genuinely and durably answered "chat is not ready" --
    // that case is a real shell with the composer disabled, never a skeleton
    // with nothing to wait for.
    mockNativeBoundary(false);
    const shellGlobal = globalThis as { __TAURI_INTERNALS__?: unknown };
    shellGlobal.__TAURI_INTERNALS__ = {};
    setAuthSession(bootstrap.session);

    try {
      render(<AppShell bootstrap={bootstrap} />);

      await waitFor(() => {
        expect(screen.getByRole("textbox", { name: "Message xFalcon" })).toBeDisabled();
      });
      expect(screen.queryByText("Loading workspace…")).not.toBeInTheDocument();
    } finally {
      delete shellGlobal.__TAURI_INTERNALS__;
      resetAuthSessionForTests();
    }
  });
  describe("inactivity sign-out", () => {
    const TWELVE_HOURS_MS = 12 * 60 * 60 * 1000;

    /** Time moves only through `Date`: the idle check reads the wall clock, and
     * leaving the timers real keeps Testing Library's own waiting working. */
    function unattendedFor(milliseconds: number) {
      vi.setSystemTime(Date.now() + milliseconds);
      // Returning to the window runs the same check the one-minute interval does.
      act(() => {
        window.dispatchEvent(new Event("focus"));
      });
    }

    function signOutIsCalled() {
      return invokeMock.mock.calls.some(([command]) => command === "auth_logout");
    }

    beforeEach(() => {
      vi.useFakeTimers({ toFake: ["Date"] });
      const defaultInvoke = invokeMock.getMockImplementation()!;
      invokeMock.mockImplementation(async (command: string, args: unknown) => {
        if (command === "auth_logout") return { status: "unauthenticated", version: 1 };
        return defaultInvoke(command, args);
      });
    });

    afterEach(() => {
      vi.useRealTimers();
      resetToastsForTests();
    });

    it("signs the person out after twelve hours without input and keeps the reason for the sign-in screen", async () => {
      setAuthSession(bootstrap.session);
      render(
        <>
          <AppShell bootstrap={bootstrap} />
          <Toaster />
        </>,
      );
      await screen.findByRole("textbox", { name: "Message xFalcon" });

      unattendedFor(TWELVE_HOURS_MS + 1_000);

      await waitFor(() => expect(signOutIsCalled()).toBe(true));
      await waitFor(() =>
        expect(getSignedOutNotice()).toBe("Signed out after 12 hours of inactivity. Sign in to continue."),
      );
      expect(screen.queryByText(/Signed out after 12 hours/)).not.toBeInTheDocument();
    });

    it("shows an error and keeps the session when logout fails, then retries five minutes later", async () => {
      setAuthSession(bootstrap.session);
      const current = invokeMock.getMockImplementation()!;
      let logoutAttempts = 0;
      invokeMock.mockImplementation(async (command: string, args: unknown) => {
        if (command === "auth_logout") {
          logoutAttempts += 1;
          if (logoutAttempts === 1) throw new Error("network down");
          return { status: "unauthenticated", version: 1 };
        }
        return current(command, args);
      });
      render(
        <>
          <AppShell bootstrap={bootstrap} />
          <Toaster />
        </>,
      );
      await screen.findByRole("textbox", { name: "Message xFalcon" });

      unattendedFor(TWELVE_HOURS_MS + 1_000);

      expect(
        await screen.findByText(
          "Could not sign you out after 12 hours of inactivity. Close xFalcon or sign out from the account menu.",
        ),
      ).toBeInTheDocument();
      expect(getSignedOutNotice()).toBeNull();
      expect(logoutAttempts).toBe(1);

      unattendedFor(5 * 60 * 1000);

      await waitFor(() => expect(logoutAttempts).toBe(2));
      await waitFor(() =>
        expect(getSignedOutNotice()).toBe("Signed out after 12 hours of inactivity. Sign in to continue."),
      );
    });

    it("postpones the sign-out while a dialog is open", async () => {
      setAuthSession(bootstrap.session);
      render(<AppShell bootstrap={bootstrap} />);
      await screen.findByRole("textbox", { name: "Message xFalcon" });
      const dialog = document.createElement("div");
      dialog.setAttribute("role", "dialog");
      document.body.appendChild(dialog);

      try {
        unattendedFor(TWELVE_HOURS_MS + 1_000);
        expect(signOutIsCalled()).toBe(false);
      } finally {
        dialog.remove();
      }

      unattendedFor(TWELVE_HOURS_MS + 1_000);
      await waitFor(() => expect(signOutIsCalled()).toBe(true));
    });

    it("does not sign out before twelve hours have passed", async () => {
      setAuthSession(bootstrap.session);
      render(<AppShell bootstrap={bootstrap} />);
      await screen.findByRole("textbox", { name: "Message xFalcon" });

      unattendedFor(TWELVE_HOURS_MS - 60_000);

      expect(signOutIsCalled()).toBe(false);
    });

    it("does not sign out while a run is still going", async () => {
      setAuthSession(bootstrap.session);
      render(<AppShell bootstrap={bootstrap} />);
      await screen.findByRole("textbox", { name: "Message xFalcon" });
      act(() => {
        setChatStoreForTests({ awaitingRun: true });
      });

      unattendedFor(TWELVE_HOURS_MS + 1_000);

      expect(signOutIsCalled()).toBe(false);
    });
  });
});
