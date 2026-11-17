import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ChatMessageRow } from "./chatStore";
import { clearTurnEdit, getTurnEditSnapshot } from "./turnEditStore";

const writeClipboardText = vi.hoisted(() => vi.fn().mockResolvedValue(undefined));

vi.mock("./clipboard", () => ({
  writeClipboardText,
}));

import { MessageRow } from "./MessageRow";

afterEach(() => {
  cleanup();
  clearTurnEdit();
});

const message: ChatMessageRow = {
  id: "m-copy",
  role: "assistant",
  text: "Copy this transcript line",
  sequence: BigInt(1),
  toolCalls: [],
};

describe("MessageRow", () => {
  it("copies message text when the copy button is pressed", async () => {
    writeClipboardText.mockClear();
    const user = userEvent.setup();
    render(<MessageRow message={message} />);

    await user.click(screen.getByRole("button", { name: "Copy message" }));

    await waitFor(() => {
      expect(writeClipboardText).toHaveBeenCalledWith(message.text);
    });
    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Copied" })).toBeInTheDocument();
    });
  });

  it("keeps a failed copy visible and offers the retry on the initiating row", async () => {
    writeClipboardText.mockClear();
    writeClipboardText.mockRejectedValueOnce(new Error("clipboard unavailable")).mockResolvedValueOnce(undefined);
    const user = userEvent.setup();
    render(<MessageRow message={message} />);

    await user.click(screen.getByRole("button", { name: "Copy message" }));

    const retry = await screen.findByRole("button", { name: "Try copy again" });
    await user.click(retry);

    await waitFor(() => expect(writeClipboardText).toHaveBeenCalledTimes(2));
    expect(await screen.findByRole("button", { name: "Copied" })).toBeInTheDocument();
  });

  it("explains the copy and regenerate icon actions on hover", async () => {
    const user = userEvent.setup();
    render(<MessageRow isLastProse message={message} onRegenerate={vi.fn()} />);

    await user.hover(screen.getByRole("button", { name: "Copy message" }));
    expect(await screen.findByRole("tooltip", { name: "Copy message" })).toBeInTheDocument();
    await user.unhover(screen.getByRole("button", { name: "Copy message" }));

    await user.hover(screen.getByRole("button", { name: "Regenerate response" }));
    expect(await screen.findByRole("tooltip", { name: "Regenerate response" })).toBeInTheDocument();
  });

  it("renders the user turn as a right-aligned bubble surface", () => {
    const { container } = render(<MessageRow message={{ ...message, id: "m-user", role: "user" }} />);
    const article = container.querySelector("article");
    expect(article?.className).toContain("justify-end");
    expect(container.querySelector(".user-bubble")).not.toBeNull();
  });

  it("marks only the latest assistant turn with the accent rail", () => {
    const { container: latest } = render(<MessageRow isLatestAssistant message={message} />);
    expect(latest.querySelector("article")?.className).toContain("assistant-accent-latest");

    const { container: history } = render(<MessageRow message={message} />);
    const historyClass = history.querySelector("article")?.className ?? "";
    expect(historyClass).toContain("assistant-accent");
    expect(historyClass).not.toContain("assistant-accent-latest");
  });

  it("keeps the ribbon's rule and the rail saying the same thing", () => {
    const { container: latest } = render(<MessageRow isLatestAssistant message={message} />);
    expect(latest.querySelector(".response-ribbon")?.className).toContain("response-ribbon--latest");

    const { container: history } = render(<MessageRow message={message} />);
    expect(history.querySelector(".response-ribbon")?.className).not.toContain("response-ribbon--latest");
  });

  it("opens every assistant answer with the response ribbon, and no user turn", () => {
    const { container: assistant } = render(<MessageRow message={message} />);
    expect(assistant.querySelector(".response-ribbon")).not.toBeNull();

    const { container: user } = render(<MessageRow message={{ ...message, id: "m-user", role: "user" }} />);
    expect(user.querySelector(".response-ribbon")).toBeNull();
  });

  it("paints the same ribbon on the in-flight row so the settle cannot flash it", () => {
    const { container } = render(<MessageRow isLatestAssistant message={{ ...message, id: "stream-run-1" }} />);
    expect(container.querySelector(".response-ribbon")).not.toBeNull();
  });

  it("trails the streaming caret only on the synthetic in-flight row", () => {
    const { container: streamRow } = render(
      <MessageRow isLatestAssistant message={{ ...message, id: "stream-run-1" }} streaming />,
    );
    expect(streamRow.querySelector(".streaming-caret")).not.toBeNull();

    const { container: settled } = render(<MessageRow isLatestAssistant message={message} />);
    expect(settled.querySelector(".streaming-caret")).toBeNull();
  });

  it("removes the caret while a completed answer waits in its synthetic row", () => {
    const live = { ...message, id: "stream-run-1" };
    const { container, rerender } = render(<MessageRow isLatestAssistant message={live} streaming />);
    expect(container.querySelector(".streaming-caret")).not.toBeNull();

    rerender(<MessageRow isLatestAssistant message={live} />);
    expect(container.querySelector(".streaming-caret")).toBeNull();
    expect(screen.getByText(message.text)).toBeInTheDocument();
  });

  it("renders Edit action on user messages and enters in-place edit mode", async () => {
    const user = userEvent.setup();
    const onEdit = vi.fn();
    const userMsg: ChatMessageRow = { ...message, id: "m-user-1", role: "user", text: "Original user question" };

    render(<MessageRow message={userMsg} onEdit={onEdit} />);

    const editBtn = screen.getByRole("button", { name: "Edit message" });
    expect(editBtn).toBeInTheDocument();

    await user.click(editBtn);
    const textarea = screen.getByRole("textbox");
    expect(textarea).toBeInTheDocument();
    expect(textarea).toHaveValue("Original user question");

    // Type new text and save
    await user.clear(textarea);
    await user.type(textarea, "Edited question");
    await user.click(screen.getByRole("button", { name: "Send" }));

    expect(onEdit).toHaveBeenCalledWith("m-user-1", "Edited question");
  });

  it("renders Regenerate action on assistant last prose row and calls onRegenerate", async () => {
    const user = userEvent.setup();
    const onRegenerate = vi.fn();
    const assistantMsg: ChatMessageRow = { ...message, id: "m-asst-1", role: "assistant", text: "Answer text" };

    render(<MessageRow isLastProse message={assistantMsg} onRegenerate={onRegenerate} />);

    const regenBtn = screen.getByRole("button", { name: "Regenerate response" });
    expect(regenBtn).toBeInTheDocument();

    await user.click(regenBtn);
    expect(onRegenerate).toHaveBeenCalledWith("m-asst-1");
  });

  it("cancels in-place editing when Cancel button or Escape is pressed", async () => {
    const user = userEvent.setup();
    const onEdit = vi.fn();
    const userMsg: ChatMessageRow = { ...message, id: "m-user-2", role: "user", text: "Question to cancel" };

    render(<MessageRow message={userMsg} onEdit={onEdit} />);

    await user.click(screen.getByRole("button", { name: "Edit message" }));
    const textarea = screen.getByRole("textbox");
    expect(textarea).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(onEdit).not.toHaveBeenCalled();
  });

  it("disables send button when text is unchanged or whitespace and does not trigger onEdit", async () => {
    const user = userEvent.setup();
    const onEdit = vi.fn();
    const userMsg: ChatMessageRow = { ...message, id: "m-user-3", role: "user", text: "Original question" };

    render(<MessageRow message={userMsg} onEdit={onEdit} />);

    await user.click(screen.getByRole("button", { name: "Edit message" }));
    const sendBtn = screen.getByRole("button", { name: "Send" });
    expect(sendBtn).toBeDisabled();

    await user.click(sendBtn);
    expect(onEdit).not.toHaveBeenCalled();

    const textarea = screen.getByRole("textbox");
    await user.clear(textarea);
    await user.type(textarea, "   ");
    expect(sendBtn).toBeDisabled();
    expect(onEdit).not.toHaveBeenCalled();
  });

  it("updates turnEditStore draft on keystroke while leaving chat snapshot unchanged", async () => {
    const user = userEvent.setup();
    const userMsg: ChatMessageRow = { ...message, id: "m-user-4", role: "user", text: "Unmodified user prompt" };

    render(<MessageRow message={userMsg} onEdit={vi.fn()} />);

    await user.click(screen.getByRole("button", { name: "Edit message" }));
    const textarea = screen.getByRole("textbox");
    await user.type(textarea, " with added criteria");

    // The turnEditStore draft is updated:
    expect(getTurnEditSnapshot().draft).toBe("Unmodified user prompt with added criteria");
    // But the message row's own props / message text remains unchanged:
    expect(userMsg.text).toBe("Unmodified user prompt");
  });

  it("draft survives the MessageRow unmounting and remounting (virtualization remount)", async () => {
    const user = userEvent.setup();
    const userMsg: ChatMessageRow = { ...message, id: "m-user-5", role: "user", text: "Initial draft base" };

    const { unmount } = render(<MessageRow message={userMsg} onEdit={vi.fn()} />);

    await user.click(screen.getByRole("button", { name: "Edit message" }));
    const textarea = screen.getByRole("textbox");
    await user.type(textarea, " plus persistent additions");
    expect(getTurnEditSnapshot().draft).toBe("Initial draft base plus persistent additions");

    // Simulate virtualization scrolling away (unmount)
    unmount();

    // Simulate virtualization scrolling back into view (remount)
    render(<MessageRow message={userMsg} onEdit={vi.fn()} />);
    const remountedTextarea = screen.getByRole("textbox");
    expect(remountedTextarea).toBeInTheDocument();
    expect(remountedTextarea).toHaveValue("Initial draft base plus persistent additions");
  });
});
