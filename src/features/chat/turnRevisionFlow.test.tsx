import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ChatConversationView,
  ChatMessageView,
  ChatSubmitResult,
  ConversationId,
} from "../../lib/generated/contracts";
import { chatMessageEdit, chatMessageRegenerate, chatMcpSelectionGet } from "../../lib/native";
import { ChatPane } from "./ChatPane";
import { applyConversationView, getChatSnapshot, resetChatStore, setChatStoreForTests } from "./chatStore";
import { clearTurnEdit } from "./turnEditStore";
import { useChat } from "./useChat";

vi.mock("../../lib/native", () => ({
  chatMcpSelectionGet: vi.fn().mockResolvedValue({ version: 1, server_id: null }),
  mcpServerList: vi.fn().mockResolvedValue({ version: 1, servers: [] }),
  diagnosticsLogRendererError: vi.fn().mockResolvedValue(undefined),
  chatConversationCreate: vi.fn(),
  chatConversationDelete: vi.fn(),
  chatConversationOpen: vi.fn(),
  chatConversationPin: vi.fn(),
  chatMessageEdit: vi.fn(),
  chatMessageRegenerate: vi.fn(),
  chatMessageSend: vi.fn(),
  chatRunCancel: vi.fn(),
  chatRunRetry: vi.fn(),
  chatWorkspaceBootstrap: vi.fn(),
  chatMessageFeedbackList: vi.fn().mockResolvedValue([]),
  chatFollowUps: vi.fn().mockResolvedValue({ suggestions: [] }),
  uploadCancel: vi.fn(),
  uploadChoose: vi.fn(),
  uploadRetry: vi.fn(),
}));

beforeEach(() => {
  vi.mocked(chatMcpSelectionGet).mockResolvedValue({ version: 1, server_id: null });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    const isScrollContainer = this instanceof HTMLDivElement && this.className.includes("overflow-y-auto");
    const height = isScrollContainer ? 480 : 72;
    return {
      width: 640,
      height,
      top: 0,
      left: 0,
      bottom: height,
      right: 640,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    };
  });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get: () => 480,
  });
});

afterEach(() => {
  cleanup();
  resetChatStore();
  clearTurnEdit();
  vi.restoreAllMocks();
});

function deferred<T>() {
  let settle!: (value: T) => void;
  const promise = new Promise<T>((resolve) => {
    settle = resolve;
  });
  return { promise, settle };
}

function message(messageId: string, role: "user" | "assistant", text: string, sequence: number): ChatMessageView {
  return {
    version: 1,
    message_id: messageId,
    role,
    text,
    sequence: BigInt(sequence),
    tool_calls: [],
    attachments: [],
  };
}

const ORIGINAL_MESSAGES = [
  message("u-1", "user", "Question one", 1),
  message("a-1", "assistant", "Answer one", 2),
  message("u-2", "user", "Question two", 3),
  message("a-2", "assistant", "Answer two", 4),
];

function conversation(
  messages: ChatMessageView[] = ORIGINAL_MESSAGES,
  activeRunId: string | null = null,
): ChatConversationView {
  return {
    version: 1,
    conversation_id: "conv-1" as ConversationId,
    title: "Revision test",
    availability: "live",
    messages,
    active_run_id: activeRunId,
  };
}

function result(status: "submitted" | "failed", messageText = ""): ChatSubmitResult {
  return {
    version: 1,
    status,
    conversation_id: status === "submitted" ? ("conv-1" as ConversationId) : null,
    run_id: status === "submitted" ? "run-revision" : null,
    message: messageText,
  };
}

function Harness() {
  const chat = useChat(true);
  return (
    <ChatPane
      activeProjectName={null}
      disabled={false}
      onClearProject={() => {}}
      onDismissNotice={chat.dismissNotice}
      onEdit={(messageId, text) => void chat.editTurn(messageId, text)}
      onOpenSettings={() => {}}
      onRegenerate={(messageId) => void chat.regenerateTurn(messageId)}
      onResetConversation={() => {}}
      onRetryRun={() => void chat.retryLastRun()}
      onStop={() => void chat.cancelRun()}
      stopping={false}
      onSubmit={(prompt) => void chat.send(prompt)}
      projectUnavailable={false}
      viewerName="Tester"
    />
  );
}

function renderConversation() {
  act(() => applyConversationView(conversation()));
  return render(<Harness />);
}

function actionFor(text: string, name: string) {
  const row = screen.getByText(text).closest("article");
  expect(row).not.toBeNull();
  return within(row as HTMLElement).getByRole("button", { name });
}

describe("turn revision through the rendered chat surface", () => {
  it("cuts an edited turn before the command resolves, then renders the durable replacement", async () => {
    vi.mocked(chatMcpSelectionGet).mockResolvedValue({ version: 1, server_id: "server-edit" });
    const edit = deferred<ChatSubmitResult>();
    vi.mocked(chatMessageEdit).mockReturnValue(edit.promise);
    const user = userEvent.setup();
    renderConversation();

    await user.click(actionFor("Question two", "Edit message"));
    const editor = screen.getByRole("textbox", { name: "Edit message" });
    const snapshotBeforeTyping = getChatSnapshot();
    await user.clear(editor);
    await user.type(editor, "Edited question two");
    expect(getChatSnapshot()).toEqual(snapshotBeforeTyping);
    await user.click(screen.getByRole("button", { name: "Send" }));

    expect(chatMessageEdit).toHaveBeenCalledWith("conv-1", "u-2", "Edited question two", "server-edit");
    expect(screen.queryByText("Question two")).toBeNull();
    expect(screen.queryByText("Answer two")).toBeNull();
    expect(screen.getByText("Answer one")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Working on the request");
    expect(actionFor("Answer one", "Regenerate response")).toBeDisabled();
    expect(screen.getByRole("button", { name: "Chat request in progress" })).toBeDisabled();

    await act(async () => edit.settle(result("submitted")));
    act(() => {
      applyConversationView(
        conversation(
          [...ORIGINAL_MESSAGES.slice(0, 2), message("u-2-revised", "user", "Edited question two", 3)],
          "run-revision",
        ),
      );
    });

    expect(screen.getByText("Edited question two")).toBeInTheDocument();
    expect(screen.queryByText("Question two")).toBeNull();
    expect(screen.queryByText("Answer two")).toBeNull();
  });

  it("restores every row and the revised draft when the edit fails", async () => {
    const edit = deferred<ChatSubmitResult>();
    vi.mocked(chatMessageEdit).mockReturnValue(edit.promise);
    const user = userEvent.setup();
    renderConversation();
    act(() => setChatStoreForTests({ activeRunId: "old-completed-run", runState: "completed" }));

    await user.click(actionFor("Question two", "Edit message"));
    const editor = screen.getByRole("textbox", { name: "Edit message" });
    await user.clear(editor);
    await user.type(editor, "Draft worth keeping");
    await user.click(screen.getByRole("button", { name: "Send" }));

    await act(async () => edit.settle(result("failed", "The revision was refused. Try again.")));

    expect(screen.getByText("Answer two")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Edit message" })).toHaveValue("Draft worth keeping");
    expect(screen.getByRole("alert")).toHaveTextContent("The revision was refused. Try again.");
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.queryByRole("button", { name: "Retry run" })).toBeNull();

    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.getByText("Question two")).toBeInTheDocument();
  });

  it("keeps the question once while regeneration removes only its response", async () => {
    const regenerate = deferred<ChatSubmitResult>();
    vi.mocked(chatMessageRegenerate).mockReturnValue(regenerate.promise);
    const user = userEvent.setup();
    renderConversation();

    await user.click(actionFor("Answer two", "Regenerate response"));

    expect(chatMessageRegenerate).toHaveBeenCalledWith("conv-1", "a-2", null);
    expect(screen.getAllByText("Question two")).toHaveLength(1);
    expect(screen.queryByText("Answer two")).toBeNull();
    expect(screen.getByRole("status")).toHaveTextContent("Working on the request");

    await act(async () => regenerate.settle(result("submitted")));
    act(() => {
      applyConversationView(conversation(ORIGINAL_MESSAGES.slice(0, 3), "run-revision"));
    });

    await waitFor(() => expect(screen.getAllByText("Question two")).toHaveLength(1));
    expect(screen.queryByText("Answer two")).toBeNull();
  });

  it("does not erase a different message's in-progress edit when a stale edit succeeds after the reader switched away", async () => {
    const edit = deferred<ChatSubmitResult>();
    vi.mocked(chatMessageEdit).mockReturnValue(edit.promise);
    const user = userEvent.setup();
    renderConversation();

    await user.click(actionFor("Question two", "Edit message"));
    await user.clear(screen.getByRole("textbox", { name: "Edit message" }));
    await user.type(screen.getByRole("textbox", { name: "Edit message" }), "Edited question two");
    await user.click(screen.getByRole("button", { name: "Send" }));
    expect(chatMessageEdit).toHaveBeenCalledWith("conv-1", "u-2", "Edited question two", null);

    // The reader switches to a different conversation before that edit answers.
    act(() => {
      applyConversationView({
        version: 1,
        conversation_id: "conv-2" as ConversationId,
        title: "Conversation two",
        availability: "live",
        messages: [message("c2-u1", "user", "A question in conversation two", 1)],
        active_run_id: null,
      });
    });

    // ...and starts editing a message there.
    await user.click(actionFor("A question in conversation two", "Edit message"));
    await user.type(screen.getByRole("textbox", { name: "Edit message" }), " plus a draft worth keeping");

    // conv-1's stale edit now reports success.
    await act(async () => edit.settle(result("submitted")));

    expect(screen.getByRole("textbox", { name: "Edit message" })).toHaveValue(
      "A question in conversation two plus a draft worth keeping",
    );
  });

  it("does not show a stale edit failure in the conversation opened while it was pending", async () => {
    const edit = deferred<ChatSubmitResult>();
    vi.mocked(chatMessageEdit).mockReturnValue(edit.promise);
    const user = userEvent.setup();
    renderConversation();

    await user.click(actionFor("Question two", "Edit message"));
    await user.click(screen.getByRole("button", { name: "Send" }));
    act(() => {
      applyConversationView({
        version: 1,
        conversation_id: "conv-2" as ConversationId,
        title: "Conversation two",
        availability: "live",
        messages: [message("c2-u1", "user", "Conversation two remains clean", 1)],
        active_run_id: null,
      });
    });

    await act(async () => edit.settle(result("failed", "Failure from conversation one")));

    expect(screen.getByText("Conversation two remains clean")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(getChatSnapshot().conversationId).toBe("conv-2");
  });

  it("does not show a stale regenerate failure in the conversation opened while it was pending", async () => {
    const regenerate = deferred<ChatSubmitResult>();
    vi.mocked(chatMessageRegenerate).mockReturnValue(regenerate.promise);
    const user = userEvent.setup();
    renderConversation();

    await user.click(actionFor("Answer two", "Regenerate response"));
    act(() => {
      applyConversationView({
        version: 1,
        conversation_id: "conv-2" as ConversationId,
        title: "Conversation two",
        availability: "live",
        messages: [message("c2-u1", "user", "Conversation two remains clean", 1)],
        active_run_id: null,
      });
    });

    await act(async () => regenerate.settle(result("failed", "Failure from conversation one")));

    expect(screen.getByText("Conversation two remains clean")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(getChatSnapshot().conversationId).toBe("conv-2");
  });
});
