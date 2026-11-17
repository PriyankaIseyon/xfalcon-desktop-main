import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ConversationList } from "./ConversationList";

const conversations = [
  {
    version: 1 as const,
    conversation_id: "conv-1",
    title: "Quarterly review",
    updated_at: "2026-01-01T00:00:00Z",
  },
  {
    version: 1 as const,
    conversation_id: "conv-2",
    title: "Budget planning",
    updated_at: "2026-01-02T00:00:00Z",
  },
  {
    version: 1 as const,
    conversation_id: "conv-3",
    title: "Hiring pipeline",
    updated_at: "2026-01-03T00:00:00Z",
  },
];

afterEach(cleanup);

describe("ConversationList rename", () => {
  it("renames a chat in place from the context menu", async () => {
    const user = userEvent.setup();
    const onRename = vi.fn().mockResolvedValue(true);
    const onSelect = vi.fn();
    render(
      <ConversationList
        activeConversationId={null}
        conversations={[conversations[0]]}
        onRename={onRename}
        onSelect={onSelect}
      />,
    );

    const title = conversations[0].title;
    await user.pointer({ keys: "[MouseRight]", target: screen.getByRole("option", { name: title }) });
    await user.click(screen.getByRole("menuitem", { name: "Rename" }));

    const field = screen.getByRole("textbox", { name: `Rename ${title}` });
    await user.clear(field);
    await user.type(field, "Q3 margin review{Enter}");

    expect(onRename).toHaveBeenCalledWith(conversations[0].conversation_id, "Q3 margin review");
    // The field replaces the row, so renaming never opens the chat as a
    // side effect of clicking into it.
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("abandons a rename on Escape without calling through", async () => {
    const user = userEvent.setup();
    const onRename = vi.fn();
    render(
      <ConversationList
        activeConversationId={null}
        conversations={[conversations[0]]}
        onRename={onRename}
        onSelect={vi.fn()}
      />,
    );

    const title = conversations[0].title;
    await user.pointer({ keys: "[MouseRight]", target: screen.getByRole("option", { name: title }) });
    await user.click(screen.getByRole("menuitem", { name: "Rename" }));
    await user.type(screen.getByRole("textbox", { name: `Rename ${title}` }), "discarded{Escape}");

    expect(onRename).not.toHaveBeenCalled();
    expect(screen.getByRole("option", { name: title })).toBeInTheDocument();
  });

  it("preserves and refocuses the draft after a failed rename, then permits retry", async () => {
    const user = userEvent.setup();
    const onRename = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true);
    render(
      <ConversationList
        activeConversationId={null}
        conversations={[conversations[0]]}
        onRename={onRename}
        onSelect={vi.fn()}
      />,
    );

    const title = conversations[0].title;
    await user.pointer({ keys: "[MouseRight]", target: screen.getByRole("option", { name: title }) });
    await user.click(screen.getByRole("menuitem", { name: "Rename" }));
    const field = screen.getByRole("textbox", { name: `Rename ${title}` });
    await user.clear(field);
    await user.type(field, "Draft worth retrying{Enter}");

    await waitFor(() => expect(field).toBeEnabled());
    expect(field).toHaveValue("Draft worth retrying");
    expect(field).toHaveFocus();

    await user.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByRole("option", { name: title })).toBeInTheDocument());
    expect(onRename).toHaveBeenCalledTimes(2);
  });
});

describe("ConversationList context menu", () => {
  it("offers pin and delete on right-click, and carries the full title as a tooltip", async () => {
    const user = userEvent.setup();
    const onPin = vi.fn();
    const onDelete = vi.fn();
    render(
      <ConversationList
        activeConversationId={null}
        conversations={[conversations[0]]}
        onDelete={onDelete}
        onPin={onPin}
        onSelect={vi.fn()}
      />,
    );

    const title = conversations[0].title;
    // Titles are auto-generated and share long prefixes, so the truncated row
    // needs the whole thing reachable somewhere. It arrives on hover, and only
    // when the text really is clipped -- a row that fits says nothing extra.
    const label = screen.getByText(title);
    expect(label).not.toHaveAttribute("title");

    Object.defineProperty(label, "clientWidth", { configurable: true, value: 40 });
    Object.defineProperty(label, "scrollWidth", { configurable: true, value: 400 });
    await user.hover(label);
    expect(label).toHaveAttribute("title", title);

    Object.defineProperty(label, "scrollWidth", { configurable: true, value: 40 });
    await user.unhover(label);
    await user.hover(label);
    expect(label).not.toHaveAttribute("title");

    await user.pointer({ keys: "[MouseRight]", target: screen.getByRole("option", { name: title }) });

    const menu = screen.getByRole("menu", { name: `Actions for ${title}` });
    await user.click(within(menu).getByRole("menuitem", { name: "Delete" }));
    expect(onDelete).toHaveBeenCalledWith(conversations[0].conversation_id);
  });
});

describe("ConversationList", () => {
  it("marks only the open conversation as the current selected option", () => {
    render(<ConversationList activeConversationId="conv-2" conversations={conversations} onSelect={vi.fn()} />);

    const open = screen.getByRole("option", { name: "Budget planning" });
    expect(open).toHaveAttribute("aria-current", "page");
    expect(open).toHaveAttribute("aria-selected", "true");

    expect(screen.getByRole("option", { name: "Quarterly review" })).toHaveAttribute("aria-selected", "false");
    expect(screen.getByRole("option", { name: "Hiring pipeline" })).toHaveAttribute("aria-selected", "false");
  });

  it("lists conversations and reports selection", async () => {
    const onSelect = vi.fn();
    const user = userEvent.setup();
    render(<ConversationList activeConversationId={null} conversations={[conversations[0]]} onSelect={onSelect} />);

    await user.click(screen.getByRole("option", { name: "Quarterly review" }));
    expect(onSelect).toHaveBeenCalledWith("conv-1");
  });

  it("moves roving focus with arrow keys, Home, and End", async () => {
    const user = userEvent.setup();
    render(<ConversationList activeConversationId={null} conversations={conversations} onSelect={vi.fn()} />);

    const first = screen.getByRole("option", { name: "Hiring pipeline" });
    const second = screen.getByRole("option", { name: "Budget planning" });
    const third = screen.getByRole("option", { name: "Quarterly review" });

    first.focus();
    expect(first).toHaveFocus();

    await user.keyboard("{ArrowDown}");
    expect(second).toHaveFocus();

    await user.keyboard("{ArrowDown}");
    expect(third).toHaveFocus();

    await user.keyboard("{ArrowDown}");
    expect(first).toHaveFocus();

    await user.keyboard("{ArrowUp}");
    expect(third).toHaveFocus();

    await user.keyboard("{Home}");
    expect(first).toHaveFocus();

    await user.keyboard("{End}");
    expect(third).toHaveFocus();
  });

  it("delimits the two groups, so a pinned chat lands under a heading that names it", () => {
    render(
      <ConversationList
        activeConversationId={null}
        conversations={[conversations[0], { ...conversations[1], pinned_at: "2026-02-01T00:00:00Z" }]}
        onSelect={vi.fn()}
      />,
    );

    // Reordering alone is not the feature. Pinning is only legible if the
    // group the chat moved INTO is labelled and its boundary is visible.
    const pinnedGroup = screen.getByRole("listbox", { name: "Pinned" });
    const olderGroup = screen.getByRole("listbox", { name: "Older" });

    expect(within(pinnedGroup).getByRole("option", { name: "Budget planning" })).toBeInTheDocument();
    expect(within(pinnedGroup).queryByRole("option", { name: "Quarterly review" })).not.toBeInTheDocument();
    expect(within(olderGroup).getByRole("option", { name: "Quarterly review" })).toBeInTheDocument();

    // The labels delimit the groups without repeating decorative icons in an
    // already dense list.
    expect(screen.getByText("Pinned").closest(".sidebar-heading")?.querySelector("svg")).toBeNull();
    expect(screen.getByText("Older").closest(".sidebar-heading")?.querySelector("svg")).toBeNull();
  });

  it("shows only the group it has rows for", () => {
    const { rerender } = render(
      <ConversationList activeConversationId={null} conversations={[conversations[0]]} onSelect={vi.fn()} />,
    );
    expect(screen.getByRole("listbox", { name: "Older" })).toBeInTheDocument();
    expect(screen.queryByText("Pinned")).not.toBeInTheDocument();

    rerender(
      <ConversationList
        activeConversationId={null}
        conversations={[{ ...conversations[0], pinned_at: "2026-02-01T00:00:00Z" }]}
        onSelect={vi.fn()}
      />,
    );
    expect(screen.getByRole("listbox", { name: "Pinned" })).toBeInTheDocument();
    // An empty recency heading under the pinned group would claim there are
    // chats it is failing to show.
    expect(screen.queryByText("Older")).not.toBeInTheDocument();
    expect(screen.queryByText("Today")).not.toBeInTheDocument();
  });

  it("keeps the heading when there is nothing to list", () => {
    render(<ConversationList activeConversationId={null} conversations={[]} onSelect={vi.fn()} />);

    expect(screen.getByText("Recents")).toBeInTheDocument();
    expect(screen.getByText("No conversations yet. Start a new chat to begin.")).toBeInTheDocument();
  });

  it("groups pinned conversations first, most recently pinned on top", () => {
    render(
      <ConversationList
        activeConversationId={null}
        conversations={[
          conversations[0],
          { ...conversations[1], pinned_at: "2026-02-01T00:00:00Z" },
          { ...conversations[2], pinned_at: "2026-03-01T00:00:00Z" },
        ]}
        onSelect={vi.fn()}
      />,
    );

    expect(screen.getByText("Pinned")).toBeInTheDocument();
    const options = screen.getAllByRole("option");
    expect(options[0]).toHaveTextContent("Hiring pipeline");
    expect(options[1]).toHaveTextContent("Budget planning");
    expect(options[2]).toHaveTextContent("Quarterly review");
  });

  it("pins, unpins, and deletes through the row actions", async () => {
    const onPin = vi.fn();
    const onDelete = vi.fn();
    const user = userEvent.setup();
    render(
      <ConversationList
        activeConversationId={null}
        conversations={[conversations[0], { ...conversations[1], pinned_at: "2026-02-01T00:00:00Z" }]}
        onDelete={onDelete}
        onPin={onPin}
        onSelect={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Pin Quarterly review" }));
    expect(onPin).toHaveBeenCalledWith("conv-1", true);

    await user.click(screen.getByRole("button", { name: "Unpin Budget planning" }));
    expect(onPin).toHaveBeenCalledWith("conv-2", false);

    await user.click(screen.getByRole("button", { name: "Delete Quarterly review" }));
    expect(onDelete).toHaveBeenCalledWith("conv-1");
  });

  it("hides the row actions when no handlers are wired", () => {
    render(<ConversationList activeConversationId={null} conversations={[conversations[0]]} onSelect={vi.fn()} />);

    expect(screen.queryByRole("button", { name: /Pin|Delete/ })).not.toBeInTheDocument();
  });
});

describe("ConversationList recency bands", () => {
  it("labels Today, Yesterday, This week, and Older from the local calendar", () => {
    const noon = (daysAgo: number) => {
      const date = new Date();
      date.setHours(12, 0, 0, 0);
      date.setDate(date.getDate() - daysAgo);
      return date.toISOString();
    };
    render(
      <ConversationList
        activeConversationId={null}
        conversations={[
          { version: 1, conversation_id: "today", title: "Today chat", updated_at: noon(0) },
          { version: 1, conversation_id: "yday", title: "Yesterday chat", updated_at: noon(1) },
          { version: 1, conversation_id: "week", title: "This week chat", updated_at: noon(4) },
          { version: 1, conversation_id: "old", title: "Older chat", updated_at: noon(20) },
        ]}
        onSelect={vi.fn()}
      />,
    );

    expect(
      within(screen.getByRole("listbox", { name: "Today" })).getByRole("option", { name: "Today chat" }),
    ).toBeInTheDocument();
    expect(
      within(screen.getByRole("listbox", { name: "Yesterday" })).getByRole("option", { name: "Yesterday chat" }),
    ).toBeInTheDocument();
    expect(
      within(screen.getByRole("listbox", { name: "This week" })).getByRole("option", { name: "This week chat" }),
    ).toBeInTheDocument();
    expect(
      within(screen.getByRole("listbox", { name: "Older" })).getByRole("option", { name: "Older chat" }),
    ).toBeInTheDocument();
  });

  it("keeps every chat visible so recents can fill the sidebar", () => {
    const many = Array.from({ length: 12 }, (_, index) => ({
      version: 1 as const,
      conversation_id: `conv-${index}`,
      title: `Chat ${index}`,
      updated_at: new Date(Date.UTC(2026, 0, index + 1)).toISOString(),
    }));
    render(<ConversationList activeConversationId={null} conversations={many} onSelect={vi.fn()} />);

    expect(screen.getByRole("option", { name: "Chat 0" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Chat 11" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /more/i })).not.toBeInTheDocument();
  });
});
