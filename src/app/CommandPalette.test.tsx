import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ChatConversationSummary, ProjectView, ReportView, SkillView } from "../lib/generated/contracts";
import { scoreSearch } from "../lib/searchText";
import { CommandPalette, type CommandPaletteProps } from "./CommandPalette";
import { MAX_RESULTS_PER_GROUP } from "./workspaceSearch";

// Wraps the real scorer, so every test still runs the real matching and only
// the calls the palette makes through this module boundary are counted.
vi.mock("../lib/searchText", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/searchText")>();
  return { ...actual, scoreSearch: vi.fn(actual.scoreSearch) };
});

afterEach(cleanup);

const SEARCH_FIELD = "Search workspace";

function conversation(id: string, title: string, pinned_at?: string): ChatConversationSummary {
  return { version: 1, conversation_id: id, title, updated_at: "2026-01-01T00:00:00Z", pinned_at };
}

function project(id: string, display_name: string, root_path = `/work/${id}`): ProjectView {
  return {
    version: 1,
    id,
    display_name,
    root_path,
    status: "trusted",
    active: false,
    trusted_at_ms: 1,
  };
}

function report(id: string, title: string, description: string | null = null): ReportView {
  return {
    version: 1,
    id,
    kind: "dashboard",
    title,
    description,
    created_at_ms: 1n,
    updated_at_ms: 1n,
    thumbnail_updated_at_ms: null,
    data_sources: [],
    pinned: false,
    hidden: false,
    byte_size: 12n,
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
}

function skill(name: string, description: string): SkillView {
  return {
    version: 1,
    name,
    description,
    declared_version: "1.0.0",
    enabled: true,
    content_hash: "a".repeat(64),
    bundled_files: 0,
  };
}

/** Every required prop defaulted, so a test states only what it is about. */
function paletteElement(props: Partial<CommandPaletteProps> = {}) {
  return (
    <CommandPalette
      conversations={[]}
      open
      onClose={vi.fn()}
      onNewChat={vi.fn()}
      onOpenConversation={vi.fn()}
      searchConversationMessages={async () => []}
      {...props}
    />
  );
}

function renderPalette(props: Partial<CommandPaletteProps> = {}) {
  return render(paletteElement(props));
}

describe("CommandPalette", () => {
  it("is absent while closed and exposes no privileged starter destinations", () => {
    renderPalette({ open: false });

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.queryByText("Settings")).not.toBeInTheDocument();
    expect(screen.queryByText("MCP")).not.toBeInTheDocument();
    expect(screen.queryByText("Skills")).not.toBeInTheDocument();
  });

  it("opens as an accessible list and closes on Escape", async () => {
    const onClose = vi.fn();
    const user = userEvent.setup();

    renderPalette({ onClose });

    const input = screen.getByRole("combobox", { name: SEARCH_FIELD });
    expect(screen.getByRole("dialog", { name: "Search" })).toBeInTheDocument();
    expect(input).toHaveFocus();
    expect(input).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByRole("option", { name: "New chat" })).toHaveAttribute("aria-selected", "true");

    await user.keyboard("{Escape}");

    expect(onClose).toHaveBeenCalledOnce();
  });

  it("selects commands with arrows and Enter", async () => {
    const onClose = vi.fn();
    const onNewChat = vi.fn();
    const user = userEvent.setup();

    renderPalette({ onClose, onNewChat });

    await user.keyboard("{ArrowDown}{Enter}");

    expect(onNewChat).toHaveBeenCalledOnce();
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("reports a genuine miss without inventing unavailable product surfaces", async () => {
    const user = userEvent.setup();

    renderPalette();
    await user.type(screen.getByRole("combobox", { name: SEARCH_FIELD }), "missing");

    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("No matches"));
    expect(screen.queryByRole("option")).not.toBeInTheDocument();
  });

  it("shows a spinner for the busy wait on message search, not text alone", async () => {
    const user = userEvent.setup();
    let resolveSearch: (ids: string[]) => void = () => {};
    const searchConversationMessages = vi.fn(
      () =>
        new Promise<string[]>((resolve) => {
          resolveSearch = resolve;
        }),
    );

    renderPalette({ searchConversationMessages });
    await user.type(screen.getByRole("combobox", { name: SEARCH_FIELD }), "missing");

    const status = await screen.findByRole("status");
    expect(status).toHaveTextContent("Searching chats…");
    // Decorative -- the visible text beside it is what a screen reader is
    // meant to read, the same split App.tsx's own startup wait uses.
    expect(status.querySelector("svg")).toHaveAttribute("aria-hidden", "true");

    // The debounce (120ms) sits between the "searching" state and the call
    // this promise belongs to -- resolving before it fires would resolve
    // nothing.
    await waitFor(() => expect(searchConversationMessages).toHaveBeenCalledWith("missing"));
    resolveSearch([]);
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("No matches"));
  });

  it("offers only the destinations the shell wired to it", async () => {
    const onOpenSettings = vi.fn();
    const onBrowseFiles = vi.fn();
    const user = userEvent.setup();

    renderPalette({ onBrowseFiles, onOpenSettings });

    // "Go to chat" was not wired here, so the palette must not advertise it.
    expect(screen.queryByRole("option", { name: "Go to chat" })).not.toBeInTheDocument();
    // Nor Reports: a listed command the build cannot reach is worse than an
    // absent one.
    expect(screen.queryByRole("option", { name: "Reports" })).not.toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Browse project artifacts" })).toBeInTheDocument();

    await user.click(screen.getByRole("option", { name: "Settings" }));

    expect(onOpenSettings).toHaveBeenCalledOnce();
    expect(onBrowseFiles).not.toHaveBeenCalled();
  });

  it("runs the Reports destination once the shell wires it", async () => {
    const onOpenReports = vi.fn();
    const user = userEvent.setup();

    renderPalette({ onOpenReports });

    // The label is the whole search surface, so it has to be the word a reader
    // types looking for a saved dashboard.
    await user.type(screen.getByRole("combobox", { name: SEARCH_FIELD }), "repo");
    await user.click(screen.getByRole("option", { name: "Reports" }));

    expect(onOpenReports).toHaveBeenCalledOnce();
  });

  it("restores focus to the opener after closing", async () => {
    function Harness() {
      const [open, setOpen] = useState(false);
      return (
        <>
          <button type="button" onClick={() => setOpen(true)}>
            Open commands
          </button>
          <CommandPalette
            conversations={[]}
            open={open}
            onClose={() => setOpen(false)}
            onNewChat={vi.fn()}
            onOpenConversation={vi.fn()}
            searchConversationMessages={async () => []}
          />
        </>
      );
    }

    const user = userEvent.setup();
    render(<Harness />);
    const opener = screen.getByRole("button", { name: "Open commands" });

    await user.click(opener);
    expect(screen.getByRole("combobox", { name: SEARCH_FIELD })).toHaveFocus();
    await user.keyboard("{Escape}");

    expect(opener).toHaveFocus();
  });

  it("traps forward and reverse tab focus inside the modal", async () => {
    const user = userEvent.setup();
    render(
      <>
        <button type="button">Outside action</button>
        <CommandPalette
          conversations={[]}
          open
          onClose={vi.fn()}
          onNewChat={vi.fn()}
          onOpenConversation={vi.fn()}
          searchConversationMessages={async () => []}
        />
      </>,
    );

    const input = screen.getByRole("combobox", { name: SEARCH_FIELD });
    const command = screen.getByRole("option", { name: "New chat" });
    expect(input).toHaveFocus();

    await user.tab();
    expect(command).toHaveFocus();
    await user.tab();
    expect(input).toHaveFocus();
    await user.tab({ shift: true });
    expect(command).toHaveFocus();
  });

  it("keeps a valid option active when the result set changes for a reason other than a keystroke", async () => {
    const user = userEvent.setup();
    const conversations = [
      conversation("conv-1", "Revenue by region"),
      conversation("conv-2", "Revenue by product"),
      conversation("conv-3", "Revenue by channel"),
    ];
    const { rerender } = renderPalette({ conversations });
    await user.type(screen.getByRole("combobox", { name: SEARCH_FIELD }), "revenue");
    await user.keyboard("{ArrowDown}{ArrowDown}");

    const options = within(screen.getByRole("group", { name: "Chats" })).getAllByRole("option");
    expect(options).toHaveLength(3);
    expect(options[2]).toHaveAttribute("aria-selected", "true");

    // The conversation list shrinks out from under the open palette -- a
    // delete or a workspace refresh elsewhere, not a keystroke here. Nothing
    // re-runs the index-to-0 reset the input's own `onChange` does.
    rerender(
      <CommandPalette
        conversations={conversations.slice(0, 1)}
        open
        onClose={vi.fn()}
        onNewChat={vi.fn()}
        onOpenConversation={vi.fn()}
        searchConversationMessages={async () => []}
      />,
    );

    await waitFor(() => {
      const remaining = within(screen.getByRole("group", { name: "Chats" })).getAllByRole("option");
      expect(remaining).toHaveLength(1);
      expect(remaining[0]).toHaveAttribute("aria-selected", "true");
    });
  });
});

describe("CommandPalette conversation search", () => {
  const conversations = [
    conversation("conv-1", "Q3 margin review"),
    conversation("conv-2", "Hiring pipeline"),
    conversation("conv-3", "Café revenue"),
  ];

  it("finds a chat by title and opens it", async () => {
    const onOpenConversation = vi.fn();
    const onClose = vi.fn();
    const user = userEvent.setup();

    renderPalette({ conversations, onClose, onOpenConversation });
    await user.type(screen.getByRole("combobox", { name: SEARCH_FIELD }), "margin");
    await user.click(screen.getByRole("option", { name: "Q3 margin review" }));

    expect(onOpenConversation).toHaveBeenCalledWith("conv-1", "margin");
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("matches without regard to case or diacritics", async () => {
    const user = userEvent.setup();

    renderPalette({ conversations });
    await user.type(screen.getByRole("combobox", { name: SEARCH_FIELD }), "CAFE");

    expect(screen.getByRole("option", { name: "Café revenue" })).toBeInTheDocument();
  });

  it("keeps commands reachable, so adding chats did not displace them", async () => {
    const user = userEvent.setup();

    renderPalette({ conversations, onOpenSettings: vi.fn() });
    await user.type(screen.getByRole("combobox", { name: SEARCH_FIELD }), "settings");

    expect(screen.getByRole("option", { name: "Settings" })).toBeInTheDocument();
  });

  it("names each group, so a row's kind is never in doubt", async () => {
    const user = userEvent.setup();

    // "Reports" is both a saved-dashboard destination and a plausible chat
    // title, which is exactly the collision the grouping exists to settle.
    renderPalette({
      conversations: [conversation("conv-9", "Reports for finance")],
      onOpenReports: vi.fn(),
    });
    await user.type(screen.getByRole("combobox", { name: SEARCH_FIELD }), "report");

    expect(within(screen.getByRole("group", { name: "Chats" })).getByRole("option")).toHaveTextContent(
      "Reports for finance",
    );
    expect(within(screen.getByRole("group", { name: "Commands" })).getByRole("option")).toHaveTextContent("Reports");
  });

  it("caps a broad query rather than out-scrolling the sidebar it replaces", async () => {
    const many = Array.from({ length: 20 }, (_, index) => conversation(`conv-${index}`, `Revenue ${index}`));
    const user = userEvent.setup();

    renderPalette({ conversations: many });
    await user.type(screen.getByRole("combobox", { name: SEARCH_FIELD }), "revenue");

    expect(within(screen.getByRole("group", { name: "Chats" })).getAllByRole("option")).toHaveLength(
      MAX_RESULTS_PER_GROUP,
    );
  });

  it("orders results the way the sidebar does, so the two never disagree", async () => {
    const user = userEvent.setup();

    renderPalette({
      conversations: [
        conversation("conv-1", "Revenue by region"),
        conversation("conv-2", "Revenue by product", "2026-02-01T00:00:00Z"),
      ],
    });
    await user.type(screen.getByRole("combobox", { name: SEARCH_FIELD }), "revenue");

    const options = within(screen.getByRole("group", { name: "Chats" })).getAllByRole("option");
    expect(options[0]).toHaveTextContent("Revenue by product");
  });

  it("opens a chat whose open transcript matches even when the title does not", async () => {
    const onOpenConversation = vi.fn();
    const user = userEvent.setup();

    renderPalette({
      conversations,
      openConversationId: "conv-2",
      openMessageTexts: ["Headcount plan for the Berlin office"],
      onOpenConversation,
    });
    await user.type(screen.getByRole("combobox", { name: SEARCH_FIELD }), "berlin");
    await user.click(screen.getByRole("option", { name: /Hiring pipeline/ }));

    expect(onOpenConversation).toHaveBeenCalledWith("conv-2", "berlin");
  });
});

describe("CommandPalette open transcript scan", () => {
  const conversations = [conversation("conv-1", "Q3 margin review"), conversation("conv-2", "Hiring pipeline")];
  const messages = ["Headcount plan for the Berlin office"];

  it("does not scan the open transcript while closed, however often the shell re-renders", async () => {
    const user = userEvent.setup();
    const props = { conversations, openConversationId: "conv-2", openMessageTexts: messages };

    const { rerender } = renderPalette(props);
    await user.type(screen.getByRole("combobox", { name: SEARCH_FIELD }), "berlin");

    // Closed, with the last query still held, and a fresh array of the same
    // messages arriving from the shell.
    rerender(paletteElement({ ...props, open: false }));
    vi.mocked(scoreSearch).mockClear();
    rerender(paletteElement({ ...props, open: false, openMessageTexts: [...messages] }));

    const transcriptScans = vi.mocked(scoreSearch).mock.calls.filter(([haystack]) => messages.includes(haystack));
    expect(transcriptScans).toHaveLength(0);
  });

  it("drops the Matched in messages hint once the query no longer matches", async () => {
    const user = userEvent.setup();

    renderPalette({ conversations, openConversationId: "conv-2", openMessageTexts: messages });
    const input = screen.getByRole("combobox", { name: SEARCH_FIELD });

    await user.type(input, "berlin");
    await waitFor(() =>
      expect(screen.getByRole("option", { name: /Hiring pipeline/ })).toHaveTextContent("Matched in messages"),
    );

    await user.clear(input);
    await user.type(input, "berlinzq");
    await waitFor(() => expect(screen.queryByRole("option", { name: /Hiring pipeline/ })).not.toBeInTheDocument());
  });
});

describe("CommandPalette workspace search", () => {
  it.each([
    ["default model", "Default model", "settings-default-model"],
    ["private models", "Private models", "settings-private-models"],
    ["model providers", "Model providers", "settings-providers"],
    ["mcp servers", "MCP servers", "settings-mcp"],
  ])("opens the %s subgroup within Models & connections", async (query, label, targetId) => {
    const onOpenSettingsSection = vi.fn();
    const user = userEvent.setup();
    renderPalette({ onOpenSettingsSection });

    await user.type(screen.getByRole("combobox", { name: SEARCH_FIELD }), query);
    await user.click(screen.getByRole("option", { name: label }));

    expect(onOpenSettingsSection).toHaveBeenCalledWith({
      section: "models-connections",
      targetId,
      query,
    });
  });

  it("finds the Try next suggestions switch in Agent capabilities", async () => {
    const onOpenSettingsSection = vi.fn();
    const user = userEvent.setup();
    renderPalette({ onOpenSettings: vi.fn(), onOpenSettingsSection });
    await user.type(screen.getByRole("combobox", { name: SEARCH_FIELD }), "try next");
    await user.click(screen.getByRole("option", { name: "Try next suggestions" }));
    expect(onOpenSettingsSection).toHaveBeenCalledWith({
      section: "agent-capabilities",
      targetId: "settings-show-follow-ups-row",
      query: "try next",
    });
  });

  it("finds a project by name and opens its files", async () => {
    const onOpenProject = vi.fn();
    const onClose = vi.fn();
    const user = userEvent.setup();

    renderPalette({
      projects: [project("proj-ida", "IDA dashboard")],
      onClose,
      onOpenProject,
    });
    await user.type(screen.getByRole("combobox", { name: SEARCH_FIELD }), "ida");
    await user.click(screen.getByRole("option", { name: /IDA dashboard/ }));

    expect(onOpenProject).toHaveBeenCalledWith("proj-ida", "ida");
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("finds a saved report by title or description", async () => {
    const onOpenReport = vi.fn();
    const user = userEvent.setup();

    renderPalette({
      reports: [report("rep-1", "Q3 pipeline", "Closed-won by region")],
      onOpenReport,
    });
    await user.type(screen.getByRole("combobox", { name: SEARCH_FIELD }), "closed-won");
    await user.click(screen.getByRole("option", { name: /Q3 pipeline/ }));

    expect(onOpenReport).toHaveBeenCalledWith("rep-1", "closed-won");
  });

  it("jumps to a settings section, a named MCP server, and a skill", async () => {
    const onOpenSettingsSection = vi.fn();
    const user = userEvent.setup();

    renderPalette({
      onOpenSettings: vi.fn(),
      onOpenSettingsSection,
      mcpServers: [{ version: 1, id: "mcp-1", name: "BigQuery", transport: "http", enabled: true }],
      skills: [skill("ida-dashboard", "Quarterly IDA pack")],
    });

    await user.type(screen.getByRole("combobox", { name: SEARCH_FIELD }), "dark");
    await user.click(screen.getByRole("option", { name: "Appearance" }));
    expect(onOpenSettingsSection).toHaveBeenCalledWith({
      section: "appearance",
      targetId: "settings-section-appearance",
      query: "dark",
    });

    await user.clear(screen.getByRole("combobox", { name: SEARCH_FIELD }));
    await user.type(screen.getByRole("combobox", { name: SEARCH_FIELD }), "bigquery");
    await user.click(screen.getByRole("option", { name: /BigQuery/ }));
    expect(onOpenSettingsSection).toHaveBeenCalledWith({
      section: "models-connections",
      targetId: "settings-mcp-mcp-1",
      query: "bigquery",
    });

    await user.clear(screen.getByRole("combobox", { name: SEARCH_FIELD }));
    await user.type(screen.getByRole("combobox", { name: SEARCH_FIELD }), "ida pack");
    await user.click(screen.getByRole("option", { name: /ida-dashboard/ }));
    expect(onOpenSettingsSection).toHaveBeenCalledWith({
      section: "agent-capabilities",
      targetId: "settings-skill-ida-dashboard",
      query: "ida pack",
    });
  });

  it("keeps the option the arrow keys select on screen, and a resting pointer cannot take it back", async () => {
    const scrolled: string[] = [];
    const scrollIntoView = vi.fn(function (this: HTMLElement) {
      scrolled.push(this.textContent ?? "");
    });
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", { configurable: true, value: scrollIntoView });
    try {
      const user = userEvent.setup();
      renderPalette({
        conversations: Array.from({ length: 6 }, (_, index) => conversation(`c${index}`, `Chat ${index}`)),
      });
      const input = screen.getByRole("combobox", { name: SEARCH_FIELD });
      const options = screen.getAllByRole("option");

      await user.keyboard("{ArrowDown}{ArrowDown}{ArrowDown}");

      const active = options[3];
      expect(active).toHaveAttribute("aria-selected", "true");
      expect(input).toHaveAttribute("aria-activedescendant", active.id);
      expect(scrolled.at(-1)).toBe(active.textContent);
      expect(scrollIntoView).toHaveBeenLastCalledWith({ block: "nearest" });

      // The list scrolled under a pointer resting on the first option: the
      // browser reports a move at the same spot, which must not select it.
      const resting = { clientX: 40, clientY: 40 };
      fireEvent.pointerMove(options[0], resting);
      fireEvent.pointerMove(options[0], resting);
      expect(options[0]).toHaveAttribute("aria-selected", "true");
      await user.keyboard("{ArrowDown}");
      fireEvent.pointerMove(options[0], resting);
      expect(options[1]).toHaveAttribute("aria-selected", "true");
    } finally {
      delete (HTMLElement.prototype as { scrollIntoView?: unknown }).scrollIntoView;
    }
  });
});
