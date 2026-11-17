import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChatPane } from "./ChatPane";
import { applyConversationView, applyChatProjectionForTests, getChatSnapshot, resetChatStore } from "./chatStore";
import { useChat } from "./useChat";
import type {
  ChatConversationView,
  ChatErrorCode,
  ChatSubmitResult,
  ConversationId,
} from "../../lib/generated/contracts";
import { chatConversationCreate, chatMessageSend, chatRunCancel, chatRunRetry } from "../../lib/native";

/**
 * The wire shape a rejected chat command actually has: Tauri serializes the
 * error *value*, so `ChatOperationError` arrives as a plain object, never an
 * `Error`. Fixtures that used `new Error(...)` could not exercise the typed
 * path at all.
 */
function chatRejection(code: ChatErrorCode, message: string) {
  return { version: 1 as const, code, message };
}

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
}));

// The transcript is virtualized, and a jsdom element has no size — without a
// measured scroll container it draws no rows at all, and this suite is about
// what the reader sees. Same fixture as `MessageList.test.tsx`.
beforeEach(() => {
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
  vi.restoreAllMocks();
});

function deferred<T>() {
  let settle!: (value: T) => void;
  let fail!: (reason: unknown) => void;
  const promise = new Promise<T>((resolve, reject) => {
    settle = resolve;
    fail = reject;
  });
  // An unhandled rejection warning would fire before the code under test
  // reaches its `catch`, so the promise always has a reader.
  promise.catch(() => {});
  return { promise, settle, fail };
}

function createdView(): ChatConversationView {
  return {
    version: 1,
    conversation_id: "conv-new" as ConversationId,
    title: "what happened last quarter?",
    availability: "live",
    // A conversation is minted before its first message is sent, so the view
    // that mints it carries none.
    messages: [],
    active_run_id: null,
  };
}

const submitted: ChatSubmitResult = {
  version: 1,
  status: "submitted",
  conversation_id: "conv-new" as ConversationId,
  run_id: "run-1",
  message: "",
};

/** The chat column wired to the real send path, as `AppShell` wires it. */
function Harness() {
  const chat = useChat(true);
  return (
    <ChatPane
      activeProjectName={null}
      disabled={false}
      onClearProject={() => {}}
      onDismissNotice={chat.dismissNotice}
      onOpenSettings={() => {}}
      onResetConversation={() => {}}
      onRetryRun={() => {
        void chat.retryLastRun();
      }}
      onStop={() => void chat.cancelRun()}
      stopping={chat.stopping}
      onSubmit={(prompt) => void chat.send(prompt)}
      projectUnavailable={false}
      viewerName="Tester"
    />
  );
}

async function sendFromComposer(prompt: string) {
  const user = userEvent.setup();
  render(<Harness />);
  await user.type(screen.getByRole("textbox", { name: "Message xFalcon" }), prompt);
  await user.click(screen.getByRole("button", { name: "Send message" }));
}

describe("a submitted turn answers the reader before the backend does", () => {
  it("draws the turn and the thinking indicator while the run is still being minted", async () => {
    const create = deferred<ChatConversationView>();
    const send = deferred<ChatSubmitResult>();
    vi.mocked(chatConversationCreate).mockReturnValue(create.promise);
    vi.mocked(chatMessageSend).mockReturnValue(send.promise);

    await sendFromComposer("what happened last quarter?");

    // Nothing has come back yet: the conversation does not exist, so no run
    // does either. Both of these are the app's own claim.
    expect(chatMessageSend).not.toHaveBeenCalled();
    expect(screen.getByText("what happened last quarter?")).toBeInTheDocument();
    expect(screen.getByText("Thinking")).toBeInTheDocument();

    const composer = screen.getByRole("textbox", { name: "Message xFalcon" });
    await userEvent.type(composer, "queue another question");
    expect(screen.getByRole("button", { name: "Chat request in progress" })).toBeDisabled();
    await userEvent.keyboard("{Enter}");
    expect(chatMessageSend).not.toHaveBeenCalled();

    // The conversation arrives empty, and must not take the turn back off the
    // screen for the length of the send.
    await act(async () => {
      create.settle(createdView());
    });
    expect(screen.getByText("what happened last quarter?")).toBeInTheDocument();
    expect(screen.getByText("Thinking")).toBeInTheDocument();

    // The run finally exists. Real run state takes the indicator over from the
    // claim with no gap between them.
    await act(async () => {
      send.settle(submitted);
    });
    expect(screen.getByText("Thinking")).toBeInTheDocument();
  });

  it("takes the turn back when the conversation could not be created", async () => {
    const create = deferred<ChatConversationView>();
    vi.mocked(chatConversationCreate).mockReturnValue(create.promise);

    await sendFromComposer("what happened last quarter?");
    expect(screen.getByText("Thinking")).toBeInTheDocument();

    await act(async () => {
      create.fail(new Error("tenant unreachable"));
    });

    // Nothing was sent, so nothing is thinking and no durable row is coming to
    // replace the one on screen.
    expect(screen.queryByText("Thinking")).toBeNull();
    expect(screen.queryByText("what happened last quarter?", { selector: "p" })).toBeNull();
    expect(screen.getByRole("textbox", { name: "Message xFalcon" })).toHaveValue("what happened last quarter?");
    expect(screen.getByText(/Could not start a new conversation/)).toBeInTheDocument();
  });

  it("takes back a failed send and restores its prompt without a fake transcript row", async () => {
    const send = deferred<ChatSubmitResult>();
    vi.mocked(chatMessageSend).mockReturnValue(send.promise);
    act(() => {
      applyConversationView({
        ...createdView(),
        conversation_id: "conv-existing" as ConversationId,
        title: "Existing conversation",
      });
    });

    await sendFromComposer("keep this prompt safe");
    expect(screen.getByText("keep this prompt safe")).toBeInTheDocument();

    await act(async () => {
      send.settle({
        version: 1,
        status: "failed",
        conversation_id: null,
        run_id: null,
        message: "The provider is unavailable. Try again.",
      });
    });

    expect(screen.queryByText("keep this prompt safe", { selector: "p" })).toBeNull();
    expect(screen.getByRole("textbox", { name: "Message xFalcon" })).toHaveValue("keep this prompt safe");
    expect(screen.getByRole("alert")).toHaveTextContent("The provider is unavailable. Try again.");
    expect(screen.queryByRole("button", { name: "Retry run" })).toBeNull();
  });
});

describe("run retry recovery", () => {
  it("keeps one busy recovery surface, survives native refusal, and gates duplicate retry", async () => {
    const retry = deferred<ChatSubmitResult>();
    vi.mocked(chatRunRetry).mockReturnValueOnce(retry.promise);
    act(() => {
      applyConversationView({
        ...createdView(),
        conversation_id: "conv-existing" as ConversationId,
        title: "Interrupted conversation",
        messages: [
          {
            version: 1,
            message_id: "message-1",
            role: "user",
            text: "try the report again",
            sequence: BigInt(1),
            tool_calls: [],
            attachments: [],
          },
        ],
        active_run_id: "run-old",
      });
      applyChatProjectionForTests({
        type: "run_state_changed",
        version: 1,
        conversation_id: "conv-existing",
        run_id: "run-old",
        state: "interrupted",
      });
    });
    const user = userEvent.setup();
    render(<Harness />);

    await user.click(screen.getByRole("button", { name: "Retry run" }));
    expect(chatRunRetry).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: "Retrying…" })).toHaveAttribute("aria-busy", "true");
    expect(screen.queryByText("Thinking")).not.toBeInTheDocument();

    await user.keyboard("{Enter}");
    expect(chatRunRetry).toHaveBeenCalledTimes(1);

    await act(async () => {
      retry.fail(new Error("native bridge unavailable"));
    });
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Could not retry the response. The secure runtime did not respond.",
    );
    expect(screen.getByRole("button", { name: "Retry run" })).toBeEnabled();

    vi.mocked(chatRunRetry).mockResolvedValueOnce({
      ...submitted,
      conversation_id: "conv-existing" as ConversationId,
      run_id: "run-new",
    });
    await user.click(screen.getByRole("button", { name: "Retry run" }));

    await screen.findByText("Thinking");
    expect(screen.queryByText(/Could not retry the response/)).not.toBeInTheDocument();
    expect(chatRunRetry).toHaveBeenCalledTimes(2);
  });
});

describe("stopping an active response", () => {
  beforeEach(() => {
    vi.mocked(chatRunCancel).mockReset();
  });

  function seedRunningResponse() {
    applyConversationView({ ...createdView(), active_run_id: "run-1" });
    applyChatProjectionForTests({
      type: "run_state_changed",
      version: 1,
      conversation_id: "conv-new",
      run_id: "run-1",
      state: "running",
    });
  }

  /**
   * Stop had no acknowledgement at all: the press fired and the control sat
   * there looking untouched until the run settled. `chat_run_cancel` is not
   * always quick — when this process holds no local handle for the run it
   * costs a `get_run`, an access reconcile, the cancel, and a reopen — so the
   * reader was left pressing a button that gave nothing back.
   */
  it("the Stop control reports that it is working while the cancel is in flight", async () => {
    seedRunningResponse();
    let settle!: () => void;
    vi.mocked(chatRunCancel).mockImplementation(
      () =>
        new Promise((resolve) => {
          settle = () => resolve(undefined);
        }),
    );
    const user = userEvent.setup();
    render(<Harness />);
    await user.click(screen.getByRole("button", { name: "Stop the assistant's response" }));

    const busy = screen.getByRole("button", { name: "Stopping the assistant's response" });
    expect(busy).toHaveAttribute("aria-busy", "true");

    await act(async () => {
      settle();
    });
    expect(screen.queryByRole("button", { name: "Stopping the assistant's response" })).not.toBeInTheDocument();
  });

  /**
   * The reader has just been told it did not stop. If the control stayed busy
   * they would have no way to ask again, which is the one thing they want.
   */
  it("the Stop control comes back after a cancel that failed", async () => {
    seedRunningResponse();
    vi.mocked(chatRunCancel).mockRejectedValueOnce(
      chatRejection("temporarily_unavailable", "The workspace is not responding right now. Try again in a moment."),
    );
    const user = userEvent.setup();
    render(<Harness />);
    await user.click(screen.getByRole("button", { name: "Stop the assistant's response" }));

    const retry = screen.getByRole("button", { name: "Stop the assistant's response" });
    expect(retry).toBeEnabled();
    expect(retry).not.toHaveAttribute("aria-busy", "true");

    vi.mocked(chatRunCancel).mockResolvedValueOnce(undefined);
    await user.click(retry);
    expect(chatRunCancel).toHaveBeenCalledTimes(2);
  });

  it("the Stop button cancels the bound run and reports a rejected request", async () => {
    seedRunningResponse();
    vi.mocked(chatRunCancel).mockRejectedValueOnce(
      chatRejection("temporarily_unavailable", "The workspace is not responding right now. Try again in a moment."),
    );
    const user = userEvent.setup();
    render(<Harness />);
    await user.click(screen.getByRole("button", { name: "Stop the assistant's response" }));
    expect(chatRunCancel).toHaveBeenCalledWith("run-1");
    expect(getChatSnapshot().error).toBe("The workspace is not responding right now. Try again in a moment.");
    expect(getChatSnapshot().runState).toBe("running");
    expect(screen.getByRole("button", { name: "Stop the assistant's response" })).toBeEnabled();
  });

  it("deduplicates Stop and ignores a late failure after completion", async () => {
    seedRunningResponse();
    let reject!: (error: Error) => void;
    vi.mocked(chatRunCancel).mockImplementation(
      () =>
        new Promise((_resolve, failed) => {
          reject = failed;
        }),
    );
    render(<Harness />);
    const stop = screen.getByRole("button", { name: "Stop the assistant's response" });
    await act(async () => {
      stop.click();
      stop.click();
    });
    expect(chatRunCancel).toHaveBeenCalledTimes(1);
    await act(async () => {
      applyChatProjectionForTests({
        type: "run_state_changed",
        version: 1,
        conversation_id: "conv-new",
        run_id: "run-1",
        state: "completed",
      });
      reject(new Error("No active run found for that id."));
    });
    expect(getChatSnapshot().error).toBeNull();
    expect(screen.queryByRole("button", { name: "Stop the assistant's response" })).not.toBeInTheDocument();
  });

  it("removes Stop when cancellation reconciliation has no active run", () => {
    seedRunningResponse();
    render(<Harness />);
    expect(screen.getByRole("button", { name: "Stop the assistant's response" })).toBeInTheDocument();

    act(() => {
      applyConversationView({ ...createdView(), active_run_id: null });
    });

    expect(screen.queryByRole("button", { name: "Stop the assistant's response" })).not.toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Message xFalcon" })).toBeEnabled();
  });

  it("Escape uses the same visible failure handling", async () => {
    seedRunningResponse();
    vi.mocked(chatRunCancel).mockRejectedValueOnce(chatRejection("failed", "No active run found for that id."));
    const user = userEvent.setup();
    render(<Harness />);
    screen.getByRole("button", { name: "Stop the assistant's response" }).focus();
    await user.keyboard("{Escape}");
    expect(chatRunCancel).toHaveBeenCalledWith("run-1");
    expect(getChatSnapshot().error).toBe("No active run found for that id.");
  });
});
