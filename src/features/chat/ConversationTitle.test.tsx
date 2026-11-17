import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ConversationTitle } from "./ConversationTitle";

afterEach(cleanup);

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((settle) => {
    resolve = settle;
  });
  return { promise, resolve };
}

describe("ConversationTitle rename", () => {
  it("cancels a changed title with the keyboard without saving it", async () => {
    const onRename = vi.fn().mockResolvedValue(true);
    const user = userEvent.setup();
    render(
      <ConversationTitle conversationId="conv-1" onRename={onRename} title="Quarterly review" subtitle="Research" />,
    );
    await user.click(screen.getByRole("button", { name: "Rename conversation" }));
    await user.type(screen.getByRole("textbox", { name: "Conversation title" }), " revised");
    await user.tab();
    await user.tab();
    expect(screen.getByRole("button", { name: "Cancel rename" })).toHaveFocus();
    await user.keyboard("{Enter}");
    await waitFor(() => expect(screen.queryByRole("textbox", { name: "Conversation title" })).toBeNull());
    expect(onRename).not.toHaveBeenCalled();
    expect(screen.getByText("Research")).toBeInTheDocument();
  });

  it("saves on leaving the edit controls and handles Escape inside them", async () => {
    const onRename = vi.fn().mockResolvedValue(true);
    const shellKeyDown = vi.fn();
    const user = userEvent.setup();
    render(
      <header onKeyDown={shellKeyDown}>
        <ConversationTitle conversationId="conv-1" onRename={onRename} title="Quarterly review" />
        <button type="button">Outside title</button>
      </header>,
    );
    await user.click(screen.getByRole("button", { name: "Rename conversation" }));
    await user.type(screen.getByRole("textbox", { name: "Conversation title" }), " revised");
    await user.tab();
    await user.tab();
    await user.tab();
    expect(screen.getByRole("button", { name: "Outside title" })).toHaveFocus();
    await waitFor(() => expect(onRename).toHaveBeenCalledWith("Quarterly review revised"));
    await waitFor(() => expect(screen.getByRole("button", { name: "Rename conversation" })).toBeInTheDocument());
    onRename.mockClear();
    await user.click(screen.getByRole("button", { name: "Rename conversation" }));
    await user.type(screen.getByRole("textbox", { name: "Conversation title" }), " discarded");
    await user.tab();
    shellKeyDown.mockClear();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("textbox", { name: "Conversation title" })).toBeNull();
    expect(onRename).not.toHaveBeenCalled();
    expect(shellKeyDown).not.toHaveBeenCalled();
  });
  it("keeps the draft focused after failure and closes only after a successful retry", async () => {
    const first = deferred<boolean>();
    const onRename = vi.fn().mockReturnValueOnce(first.promise).mockResolvedValueOnce(true);
    const user = userEvent.setup();
    render(<ConversationTitle conversationId="conv-1" onRename={onRename} title="Quarterly review" />);

    await user.click(screen.getByRole("button", { name: "Rename conversation" }));
    const input = screen.getByRole("textbox", { name: "Conversation title" });
    await user.clear(input);
    await user.type(input, "Q3 margin review{Enter}");

    expect(input).toBeDisabled();
    expect(input).toHaveAttribute("aria-busy", "true");
    await act(async () => first.resolve(false));

    await waitFor(() => expect(input).toBeEnabled());
    expect(input).toHaveValue("Q3 margin review");
    expect(input).toHaveFocus();

    await user.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByText("Quarterly review")).toBeInTheDocument());
    expect(onRename).toHaveBeenNthCalledWith(1, "Q3 margin review");
    expect(onRename).toHaveBeenNthCalledWith(2, "Q3 margin review");
  });

  it("does not let a late rename result reopen a draft after the conversation changes", async () => {
    const pending = deferred<boolean>();
    const user = userEvent.setup();
    const { rerender } = render(
      <ConversationTitle conversationId="conv-1" onRename={() => pending.promise} title="First conversation" />,
    );
    await user.click(screen.getByRole("button", { name: "Rename conversation" }));
    await user.type(screen.getByRole("textbox", { name: "Conversation title" }), " revised{Enter}");

    rerender(
      <ConversationTitle conversationId="conv-2" onRename={() => Promise.resolve(true)} title="Second conversation" />,
    );
    await act(async () => pending.resolve(false));

    expect(screen.getByText("Second conversation")).toBeInTheDocument();
    expect(screen.queryByRole("textbox", { name: "Conversation title" })).toBeNull();
  });
});
