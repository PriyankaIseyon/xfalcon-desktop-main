import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ChatConversationSummary, ProjectView } from "../../lib/generated/contracts";
import { NO_CHAT_FILES } from "./chatFiles";
import { ProjectNavList } from "./ProjectNavList";

afterEach(cleanup);

function project(id: string, displayName: string): ProjectView {
  return {
    version: 1,
    id,
    display_name: displayName,
    root_path: `/tmp/${id}`,
    status: "trusted",
    active: false,
    trusted_at_ms: 0,
  };
}

function conversation(id: string, title: string, projectId: string): ChatConversationSummary {
  return {
    version: 1,
    conversation_id: id,
    title,
    project_id: projectId,
    updated_at: "2026-01-01T00:00:00Z",
    pinned_at: null,
  } as ChatConversationSummary;
}

function renderList(overrides: Partial<React.ComponentProps<typeof ProjectNavList>> = {}) {
  const props: React.ComponentProps<typeof ProjectNavList> = {
    projects: [project("p1", "Quarterly model"), project("p2", "Pricing study")],
    activeProjectId: undefined,
    conversationsFor: () => [],
    activeConversationId: null,
    activeConversationFiles: NO_CHAT_FILES,
    chatReady: true,
    pane: "files",
    openFilePath: null,
    onBrowseFiles: vi.fn(),
    onStartProjectChat: vi.fn(),
    onOpenFile: vi.fn(),
    onOpenConversation: vi.fn(),
    onPinConversation: vi.fn(),
    onDeleteConversation: vi.fn(),
    onRenameConversation: vi.fn().mockResolvedValue(true),
    onProjectReady: vi.fn(),
    onRemoveProject: vi.fn(),
    ...overrides,
  };
  render(<ProjectNavList {...props} />);
  return props;
}

/** The row's own label, without its hover actions. */
function rowNames(): string[] {
  return screen.getAllByRole("treeitem").map((row) => row.textContent?.trim() ?? "");
}

const FILES = {
  files: [
    { path: "src/pricing/elasticity.ts", sensitive: false },
    { path: "notes/margin.md", sensitive: false },
  ],
  hidden: 0,
};

describe("ProjectNavList", () => {
  // The whole point of replacing the dropdown: every project is visible
  // without opening anything.
  it("names every project at once", () => {
    renderList();

    expect(screen.getByRole("treeitem", { name: "Quarterly model" })).toBeInTheDocument();
    expect(screen.getByRole("treeitem", { name: "Pricing study" })).toBeInTheDocument();
  });

  /*
   * The bug this list was rebuilt around. Pressing a project's name started a
   * new conversation in it and left for the chat surface: the open chat was
   * replaced by an empty one, and the column the reader was working in was
   * swapped for another destination's. A container row opens and closes.
   */
  it("opens a project in place instead of navigating away from it", async () => {
    const user = userEvent.setup();
    const props = renderList();

    const row = screen.getByRole("treeitem", { name: "Quarterly model" });
    expect(row).toHaveAttribute("aria-expanded", "false");

    await user.click(row);

    expect(screen.getByRole("treeitem", { name: "Quarterly model" })).toHaveAttribute("aria-expanded", "true");
    expect(props.onStartProjectChat).not.toHaveBeenCalled();
    expect(props.onBrowseFiles).not.toHaveBeenCalled();
  });

  // A chat started in a project belongs under that project, which is the whole
  // reason this is a tree and not a list.
  it("carries a project's own chats under it", async () => {
    const user = userEvent.setup();
    renderList({ conversationsFor: (id) => (id === "p1" ? [conversation("c1", "Margin walk", "p1")] : []) });

    await user.click(screen.getByRole("treeitem", { name: "Quarterly model" }));

    expect(rowNames()).toEqual(["Quarterly model", "Margin walk", "Browse files", "Pricing study"]);
  });

  it("opens one of those chats, and asks for it by id", async () => {
    const user = userEvent.setup();
    const props = renderList({
      activeProjectId: "p1",
      conversationsFor: (id) => (id === "p1" ? [conversation("c1", "Margin walk", "p1")] : []),
    });

    await user.click(screen.getByRole("treeitem", { name: /Margin walk/ }));

    expect(props.onOpenConversation).toHaveBeenCalledWith("c1");
  });

  it("shows a project's files from the row under it", async () => {
    const user = userEvent.setup();
    const props = renderList();

    await user.click(screen.getByRole("treeitem", { name: "Quarterly model" }));
    await user.click(screen.getByRole("treeitem", { name: "Browse files" }));

    expect(props.onBrowseFiles).toHaveBeenCalledWith("p1");
  });

  // The grain the tree exists for: a chat's own files hang off the chat, not
  // off the project.
  it("nests the open chat's files under that chat, above the project's own file row", () => {
    renderList({
      activeProjectId: "p1",
      activeConversationId: "c1",
      activeConversationFiles: FILES,
      conversationsFor: (id) => (id === "p1" ? [conversation("c1", "Margin walk", "p1")] : []),
    });

    expect(rowNames()).toEqual([
      "Quarterly model",
      "Margin walk",
      "elasticity.ts",
      "margin.md",
      "Browse files",
      "Pricing study",
    ]);
  });

  it("opens a file row against the project that owns it", async () => {
    const user = userEvent.setup();
    const props = renderList({
      activeProjectId: "p1",
      activeConversationId: "c1",
      activeConversationFiles: FILES,
      conversationsFor: (id) => (id === "p1" ? [conversation("c1", "Margin walk", "p1")] : []),
    });

    await user.click(screen.getByRole("treeitem", { name: "margin.md" }));

    expect(props.onOpenFile).toHaveBeenCalledWith("p1", "notes/margin.md");
  });

  /*
   * One mark, following what the destination is actually showing. Two separate
   * booleans used to answer this, and could both say yes at once.
   */
  it("marks the chat row current while the chat is the pane, and the file row while the files are", () => {
    const props = {
      activeProjectId: "p1",
      activeConversationId: "c1" as const,
      activeConversationFiles: FILES,
      conversationsFor: (id: string) => (id === "p1" ? [conversation("c1", "Margin walk", "p1")] : []),
      openFilePath: "notes/margin.md",
    };

    renderList({ ...props, pane: "chat" });
    let current = screen.getAllByRole("treeitem").filter((row) => row.getAttribute("aria-current") === "page");
    expect(current).toHaveLength(1);
    expect(current[0]).toHaveTextContent("Margin walk");

    cleanup();
    renderList({ ...props, pane: "files" });
    current = screen.getAllByRole("treeitem").filter((row) => row.getAttribute("aria-current") === "page");
    expect(current).toHaveLength(1);
    expect(current[0]).toHaveTextContent("margin.md");
  });

  // A file opened from the project's own tree has no row of its own here, and
  // leaving nothing marked would tell the reader they are nowhere.
  it("falls back to the project's file row for a file no chat row names", () => {
    renderList({ activeProjectId: "p1", openFilePath: "vendor/README.md" });

    const current = screen.getAllByRole("treeitem").filter((row) => row.getAttribute("aria-current") === "page");
    expect(current).toHaveLength(1);
    expect(current[0]).toHaveTextContent("Browse files");
  });

  /*
   * The only way to start a chat in a project used to be its context menu --
   * an action you can only find if you already believe it is there. Every
   * project row carries it now, on the row it belongs to.
   */
  it("puts a new chat on every project's own row", async () => {
    const user = userEvent.setup();
    const props = renderList();

    await user.click(screen.getByRole("button", { name: "New chat in Pricing study" }));

    expect(props.onStartProjectChat).toHaveBeenCalledWith("p2");
  });

  // The row underneath opens and closes the project. The button on it must not
  // do both things at once.
  it("starts that chat without opening or closing the project", async () => {
    const user = userEvent.setup();
    renderList();

    await user.click(screen.getByRole("button", { name: "New chat in Quarterly model" }));

    expect(screen.getByRole("treeitem", { name: /Quarterly model/ })).toHaveAttribute("aria-expanded", "false");
  });

  // The same gate every other chat action in this column sits behind: nothing
  // starts a conversation before the backend can hold one.
  it("withholds a new chat until the backend is ready", () => {
    renderList({ chatReady: false });

    expect(screen.getByRole("button", { name: "New chat in Quarterly model" })).toBeDisabled();
  });

  // The keyboard's route to the same action, since the button is reachable
  // only by pointer.
  it("keeps the new chat in the project's menu as well", async () => {
    const user = userEvent.setup();
    const props = renderList();

    await user.pointer({ keys: "[MouseRight]", target: screen.getByRole("treeitem", { name: /Quarterly model/ }) });
    await user.click(screen.getByRole("menuitem", { name: "New chat in this project" }));

    expect(props.onStartProjectChat).toHaveBeenCalledWith("p1");
  });

  it("says plainly when a project has no chats", async () => {
    const user = userEvent.setup();
    renderList();

    await user.click(screen.getByRole("treeitem", { name: "Quarterly model" }));

    expect(screen.getByText("No chats in this project yet.")).toBeInTheDocument();
  });

  // The active project is what you are working in; finding it collapsed would
  // be the sidebar disagreeing with the view beside it.
  it("opens the active project without being asked", () => {
    renderList({ activeProjectId: "p2" });

    expect(screen.getByRole("treeitem", { name: "Pricing study" })).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByRole("treeitem", { name: "Quarterly model" })).toHaveAttribute("aria-expanded", "false");
  });

  it("keeps the active project collapsed once it is closed by hand", async () => {
    const user = userEvent.setup();
    renderList({ activeProjectId: "p1" });

    await user.click(screen.getByRole("treeitem", { name: "Quarterly model" }));

    expect(screen.getByRole("treeitem", { name: "Quarterly model" })).toHaveAttribute("aria-expanded", "false");
  });

  it("offers one control to expand and collapse every project", async () => {
    const user = userEvent.setup();
    renderList({ conversationsFor: (id) => [conversation(`chat-${id}`, "Margin walk", id)] });

    await user.click(screen.getByRole("button", { name: "Expand all projects" }));
    expect(screen.getAllByRole("treeitem", { name: /Margin walk/ })).toHaveLength(2);

    await user.click(screen.getByRole("button", { name: "Collapse all projects" }));
    expect(screen.queryByRole("treeitem", { name: /Margin walk/ })).not.toBeInTheDocument();
  });

  // The list IS the Projects destination's column now, so there is no title to
  // fold it under -- and the row where that title used to sit carries the one
  // action that fills it instead.
  it("offers New project where the section title used to be, and no fold", () => {
    renderList();

    expect(screen.getByRole("button", { name: "New project" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Projects section/ })).not.toBeInTheDocument();
    expect(window.localStorage.getItem("xf.sidebar.section.projects")).toBeNull();
    expect(screen.getByRole("tree", { name: "Projects" })).toBeInTheDocument();
  });

  it("puts the create action and the density control on one row, and names the tree itself", () => {
    renderList();

    const create = screen.getByRole("button", { name: "New project" });
    const density = screen.getByRole("button", { name: /all projects$/ });
    expect(create.closest(".app-sidebar__heading")).toBe(density.closest(".app-sidebar__heading"));
    expect(screen.getByRole("tree", { name: "Projects" })).toBeInTheDocument();
  });

  it("says plainly when there are no projects", () => {
    renderList({ projects: [] });

    expect(screen.getByText("No projects yet.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "New project" })).toBeInTheDocument();
    expect(screen.queryByRole("tree")).toBeNull();
  });

  // Depth is one number per row, and every cue reads from it. `aria-level` is
  // the one a screen reader hears, so it is the one worth pinning.
  it("states each row's depth: project, then chat, then file", () => {
    renderList({
      activeProjectId: "p1",
      activeConversationId: "c1",
      activeConversationFiles: FILES,
      conversationsFor: (id) => (id === "p1" ? [conversation("c1", "Margin walk", "p1")] : []),
    });

    expect(screen.getByRole("treeitem", { name: "Quarterly model" })).toHaveAttribute("aria-level", "1");
    expect(screen.getByRole("treeitem", { name: /Margin walk/ })).toHaveAttribute("aria-level", "2");
    expect(screen.getByRole("treeitem", { name: "elasticity.ts" })).toHaveAttribute("aria-level", "3");
  });

  it("moves a keyboard cursor between visible rows and opens with the arrow keys", async () => {
    const user = userEvent.setup();
    const props = renderList();

    const tree = screen.getByRole("tree");
    tree.focus();
    await user.keyboard("{ArrowRight}");

    // Right opens the container rather than moving, which is the tree contract.
    expect(screen.getByRole("treeitem", { name: "Quarterly model" })).toHaveAttribute("aria-expanded", "true");

    await user.keyboard("{ArrowDown}{ArrowDown}{Enter}");
    expect(props.onBrowseFiles).toHaveBeenCalledWith("p1");
  });

  it("opens a row's menu from the keyboard, named after that row", async () => {
    const user = userEvent.setup();
    renderList();

    screen.getByRole("tree").focus();
    await user.keyboard("{Shift>}{F10}{/Shift}");

    // The cursor starts on the first row, so its menu is the one that opens.
    expect(screen.getByRole("menu", { name: "Actions for Quarterly model" })).toBeInTheDocument();
  });

  /*
   * Removing a project is the only irreversible thing here, and "remove" is the
   * honest word: the trust record goes, the folder does not.
   */
  it("confirms a removal, names the project, and says what survives", async () => {
    const user = userEvent.setup();
    const props = renderList();

    await user.click(screen.getByRole("button", { name: "Remove Quarterly model" }));

    expect(screen.getByRole("alertdialog", { name: "Remove this project?" })).toBeInTheDocument();
    expect(screen.getByText(/The folder and every file in it stay on this computer/)).toBeInTheDocument();
    expect(screen.getByText(/Existing chats are not deleted/)).toBeInTheDocument();
    expect(props.onRemoveProject).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(props.onRemoveProject).not.toHaveBeenCalled();
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Remove Quarterly model" }));

    await user.click(screen.getByRole("button", { name: "Remove project" }));
    await waitFor(() => expect(props.onRemoveProject).toHaveBeenCalledWith("p1"));
  });

  it("keeps removal blocked and retryable when revoking trust fails", async () => {
    let rejectRemoval: (cause: Error) => void = () => undefined;
    const onRemoveProject = vi
      .fn()
      .mockReturnValueOnce(
        new Promise<void>((_resolve, reject) => {
          rejectRemoval = reject;
        }),
      )
      .mockResolvedValueOnce(undefined);
    const user = userEvent.setup();
    renderList({ onRemoveProject });
    await user.click(screen.getByRole("button", { name: "Remove Quarterly model" }));

    await user.click(screen.getByRole("button", { name: "Remove project" }));
    expect(screen.getByRole("button", { name: "Remove project" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
    await act(async () => rejectRemoval(new Error("trust store is locked")));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "This project could not be removed. Trust store is locked.",
    );
    expect(screen.getByRole("alertdialog", { name: "Remove this project?" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Remove project" }));
    await waitFor(() => expect(onRemoveProject).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
  });

  it("does not open the project it is removing", async () => {
    const user = userEvent.setup();
    renderList();

    await user.click(screen.getByRole("button", { name: "Remove Quarterly model" }));

    expect(screen.getByRole("treeitem", { name: "Quarterly model" })).toHaveAttribute("aria-expanded", "false");
  });

  it("renames one of a project's chats in place", async () => {
    const user = userEvent.setup();
    const props = renderList({
      activeProjectId: "p1",
      conversationsFor: (id) => (id === "p1" ? [conversation("c1", "Margin walk", "p1")] : []),
    });

    await user.pointer({ keys: "[MouseRight]", target: screen.getByRole("treeitem", { name: /Margin walk/ }) });
    await user.click(screen.getByRole("menuitem", { name: "Rename" }));

    const input = screen.getByRole("textbox", { name: "Rename Margin walk" });
    await user.clear(input);
    await user.type(input, "Margin review{Enter}");

    expect(props.onRenameConversation).toHaveBeenCalledWith("c1", "Margin review");
    await waitFor(() => expect(screen.queryByRole("textbox", { name: "Rename Margin walk" })).not.toBeInTheDocument());
    expect(screen.getByRole("treeitem", { name: /Margin walk/ })).toBeInTheDocument();
  });

  it("keeps a rejected rename open on the typed draft instead of discarding it", async () => {
    let resolveRename: (succeeded: boolean) => void = () => undefined;
    const onRenameConversation = vi.fn().mockReturnValue(
      new Promise<boolean>((resolve) => {
        resolveRename = resolve;
      }),
    );
    const user = userEvent.setup();
    renderList({
      activeProjectId: "p1",
      conversationsFor: (id) => (id === "p1" ? [conversation("c1", "Margin walk", "p1")] : []),
      onRenameConversation,
    });

    await user.pointer({ keys: "[MouseRight]", target: screen.getByRole("treeitem", { name: /Margin walk/ }) });
    await user.click(screen.getByRole("menuitem", { name: "Rename" }));
    const input = screen.getByRole("textbox", { name: "Rename Margin walk" });
    await user.clear(input);
    await user.type(input, "Margin review{Enter}");

    expect(onRenameConversation).toHaveBeenCalledWith("c1", "Margin review");
    // The handler already reports the reason through a toast (`AppShell`'s
    // own concern); this row's job is only to not lose what was typed.
    await act(async () => resolveRename(false));
    expect(screen.getByRole("textbox", { name: "Rename Margin walk" })).toHaveValue("Margin review");

    // A retry with the same editor still open must work.
    onRenameConversation.mockResolvedValueOnce(true);
    await user.keyboard("{Enter}");
    await waitFor(() => expect(onRenameConversation).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.queryByRole("textbox", { name: "Rename Margin walk" })).not.toBeInTheDocument());
  });
});
