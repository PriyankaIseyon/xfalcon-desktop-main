import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ChatConversationView,
  ChatErrorCode,
  ChatSubmitResult,
  ChatWorkspaceBootstrap,
  ConversationId,
} from "../../lib/generated/contracts";
import {
  chatConversationCreate,
  chatConversationOpen,
  chatConversationPreview,
  chatMcpSelectionGet,
  chatMessageSend,
  chatWorkspaceBootstrap,
} from "../../lib/native";
import {
  adoptActiveRun,
  applyChatProjectionForTests,
  applyConversationView,
  applyInferenceForTests,
  applyToolInvocationForTests,
  getChatSnapshot,
  resetChatStore,
  setChatError,
  setChatStoreForTests,
  setWorkspaceBootstrap,
} from "./chatStore";
import { listen } from "@tauri-apps/api/event";
import { resetAuthSessionForTests } from "../auth/authStore";
import { getComposerDraft, resetComposerDraft, setComposerDraft } from "./composerStore";
import { useChat } from "./useChat";

/**
 * The wire shape a rejected chat command actually has: Tauri serializes the
 * error *value*, so `ChatOperationError` arrives as a plain object, never an
 * `Error`.
 */
function chatRejection(code: ChatErrorCode, message: string) {
  return { version: 1 as const, code, message };
}

// The send path stubs the Tauri global so the workspace refresh runs; the
// chat event bridge starts with it and needs a listener to attach to.
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));

vi.mock("../../lib/native", () => ({
  chatConversationCreate: vi.fn(),
  chatConversationDelete: vi.fn(),
  chatConversationOpen: vi.fn(),
  chatConversationPreview: vi.fn(),
  chatMcpSelectionGet: vi.fn().mockResolvedValue({ version: 1, server_id: null }),
  chatConversationPin: vi.fn(),
  chatMessageEdit: vi.fn(),
  chatMessageRegenerate: vi.fn(),
  chatMessageSend: vi.fn(),
  chatRunCancel: vi.fn(),
  chatRunRetry: vi.fn(),
  chatWorkspaceBootstrap: vi.fn(),
  uploadCancel: vi.fn(),
  uploadChoose: vi.fn(),
  uploadRetry: vi.fn(),
}));

beforeEach(() => {
  vi.mocked(chatMcpSelectionGet).mockResolvedValue({ version: 1, server_id: null });
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  promise.catch(() => {});
  return { promise, resolve, reject };
}

function view(id: string, title: string, text: string, availability: "live" | "stale" = "live"): ChatConversationView {
  return {
    version: 1,
    conversation_id: id as ConversationId,
    title,
    availability,
    messages: text
      ? [
          {
            version: 1,
            message_id: `${id}-m`,
            role: "assistant",
            text,
            sequence: BigInt(1),
            tool_calls: [],
            attachments: [],
          },
        ]
      : [],
    active_run_id: null,
  };
}

function seed() {
  setWorkspaceBootstrap([
    { version: 1, conversation_id: "conv-0", title: "Settled", updated_at: "2026-09-01T00:00:00Z" },
    { version: 1, conversation_id: "conv-a", title: "Alpha", updated_at: "2026-09-01T00:00:00Z" },
    { version: 1, conversation_id: "conv-b", title: "Beta", updated_at: "2026-09-01T00:00:00Z" },
  ]);
  applyConversationView(view("conv-0", "Settled", "original"));
}

afterEach(() => {
  cleanup();
  resetChatStore();
  vi.clearAllMocks();
});

describe("local-first conversation navigation", () => {
  it("restores the native selection, keeps a same choice in place, and starts a clean chat for a changed choice", async () => {
    seed();
    vi.mocked(chatMcpSelectionGet).mockResolvedValue({ version: 1, server_id: "server-a" });
    setComposerDraft("draft from old chat", "conv-0");
    const { result } = renderHook(() => useChat(true, false));
    await waitFor(() => expect(result.current.mcpSelection).toMatchObject({ serverId: "server-a", status: "ready" }));

    await act(async () => result.current.selectMcpServer("server-a"));
    expect(getChatSnapshot().conversationId).toBe("conv-0");
    expect(getChatSnapshot().messages.map((message) => message.text)).toEqual(["original"]);

    await act(async () => result.current.selectMcpServer("server-b"));
    expect(getChatSnapshot()).toMatchObject({
      conversationId: null,
      messages: [],
      activeRunId: null,
      activity: [],
      reasoning: [],
    });
    expect(getChatSnapshot().conversations.map((row) => row.conversation_id)).toContain("conv-0");
    expect(getComposerDraft(null)).toBe("");
    expect(getComposerDraft("conv-0")).toBe("");
    expect(result.current.mcpSelection).toMatchObject({ conversationId: null, serverId: "server-b", status: "ready" });
  });

  it("blocks selection changes during a pending run and turn revision", async () => {
    seed();
    vi.mocked(chatMcpSelectionGet).mockResolvedValue({ version: 1, server_id: "server-a" });
    const { result } = renderHook(() => useChat(true, false));
    await waitFor(() => expect(result.current.mcpSelection.status).toBe("ready"));

    act(() => setChatStoreForTests({ activeRunId: "run-active", runState: "awaiting_approval" }));
    await act(async () => result.current.selectMcpServer("server-b"));
    expect(getChatSnapshot().conversationId).toBe("conv-0");
    expect(result.current.mcpSelection.serverId).toBe("server-a");

    act(() => setChatStoreForTests({ activeRunId: null, runState: "completed", revising: true }));
    await act(async () => result.current.selectMcpServer("server-b"));
    expect(getChatSnapshot().conversationId).toBe("conv-0");
    expect(result.current.mcpSelection.serverId).toBe("server-a");
  });

  it("preserves the complete chat and draft when project deactivation fails", async () => {
    seed();
    vi.mocked(chatMcpSelectionGet).mockResolvedValue({ version: 1, server_id: "server-a" });
    const { result } = renderHook(() => useChat(true, false));
    await waitFor(() => expect(result.current.mcpSelection.status).toBe("ready"));
    setComposerDraft("keep this draft", "conv-0");
    const before = getChatSnapshot();

    await act(async () => result.current.selectMcpServer("server-b", async () => false));

    expect(getChatSnapshot()).toEqual(before);
    expect(getComposerDraft("conv-0")).toBe("keep this draft");
    expect(result.current.mcpSelection).toMatchObject({ serverId: "server-a", status: "ready" });
  });

  it("does not reset a newer conversation when project deactivation resolves late", async () => {
    seed();
    vi.mocked(chatMcpSelectionGet)
      .mockResolvedValueOnce({ version: 1, server_id: "server-a" })
      .mockResolvedValueOnce({ version: 1, server_id: "server-b" });
    vi.mocked(chatConversationPreview).mockResolvedValue(null);
    vi.mocked(chatConversationOpen).mockResolvedValue(view("conv-b", "Beta", "newer conversation"));
    const prepare = deferred<boolean>();
    const { result } = renderHook(() => useChat(true, false));
    await waitFor(() => expect(result.current.mcpSelection.serverId).toBe("server-a"));

    let pending!: Promise<void>;
    act(() => {
      pending = result.current.selectMcpServer("server-new", () => prepare.promise);
    });
    await act(async () => result.current.openConversation("conv-b" as ConversationId));
    await waitFor(() => expect(result.current.mcpSelection).toMatchObject({ serverId: "server-b", status: "ready" }));
    await act(async () => {
      prepare.resolve(true);
      await pending;
    });

    expect(getChatSnapshot().conversationId).toBe("conv-b");
    expect(getChatSnapshot().messages.map((message) => message.text)).toEqual(["newer conversation"]);
    expect(result.current.mcpSelection.serverId).toBe("server-b");
  });

  it("does not apply a pending server choice after the account changes on a new-chat screen", async () => {
    const prepare = deferred<boolean>();
    const oldHook = renderHook(() => useChat(true, false));
    let pending!: Promise<void>;
    act(() => {
      pending = oldHook.result.current.selectMcpServer("old-account-server", () => prepare.promise);
    });

    act(() => {
      resetAuthSessionForTests();
      resetChatStore();
    });
    oldHook.unmount();
    const currentHook = renderHook(() => useChat(true, false));
    await waitFor(() =>
      expect(currentHook.result.current.mcpSelection).toMatchObject({ serverId: null, status: "ready" }),
    );
    setComposerDraft("current account draft", null);
    await act(async () => {
      prepare.resolve(true);
      await pending;
    });

    expect(getChatSnapshot().conversationId).toBeNull();
    expect(currentHook.result.current.mcpSelection).toMatchObject({ serverId: null, status: "ready" });
    expect(getComposerDraft(null)).toBe("current account draft");
    vi.mocked(chatConversationCreate).mockResolvedValue(view("conv-new", "New", ""));
    vi.mocked(chatMessageSend).mockResolvedValue({
      version: 1,
      status: "submitted",
      conversation_id: "conv-new" as ConversationId,
      run_id: "run-current-account",
      message: "",
    });
    await act(async () => currentHook.result.current.send("current account question"));
    expect(chatConversationCreate).toHaveBeenCalledWith("current account question", null);
    expect(chatMessageSend).toHaveBeenCalledWith("conv-new", "current account question", [], null);
  });

  it("preserves a newer same-account new-chat draft when prepare resolves late", async () => {
    const prepare = deferred<boolean>();
    vi.mocked(chatConversationCreate).mockResolvedValue(view("conv-new", "New", ""));
    vi.mocked(chatMessageSend).mockResolvedValue({
      version: 1,
      status: "submitted",
      conversation_id: "conv-new" as ConversationId,
      run_id: "run-current-draft",
      message: "",
    });
    const { result } = renderHook(() => useChat(true, false));
    let pending!: Promise<void>;
    act(() => {
      pending = result.current.selectMcpServer("stale-server", () => prepare.promise);
    });

    await act(async () => result.current.startNewConversation());
    setComposerDraft("new draft stays", null);
    await act(async () => {
      prepare.resolve(true);
      await pending;
    });

    expect(result.current.mcpSelection).toMatchObject({ serverId: null, status: "ready" });
    expect(getComposerDraft(null)).toBe("new draft stays");
    await act(async () => result.current.send("usable current draft"));
    expect(chatConversationCreate).toHaveBeenCalledWith("usable current draft", null);
    expect(chatMessageSend).toHaveBeenCalledWith("conv-new", "usable current draft", [], null);
  });

  it("clears prior live activity, reasoning, prose, and invocation admission on a changed selection", async () => {
    seed();
    vi.mocked(chatMcpSelectionGet).mockResolvedValue({ version: 1, server_id: "server-a" });
    const { result } = renderHook(() => useChat(true, false));
    await waitFor(() => expect(result.current.mcpSelection.status).toBe("ready"));

    act(() => {
      adoptActiveRun("conv-0", "run-old");
      applyToolInvocationForTests({
        type: "requested",
        version: 1,
        invocation_id: "inv-old",
        run_id: "run-old",
        tool_id: "mcp.server.tool",
      });
      setChatStoreForTests({
        activeRunId: null,
        runState: "completed",
        roundProse: [{ text: "old round", afterCallCount: 0 }],
        reasoning: [{ text: "old reasoning", afterCallCount: 0 }],
        reasoningTrimmed: true,
      });
    });
    expect(getChatSnapshot().activity).toHaveLength(1);

    await act(async () => result.current.selectMcpServer("server-b"));
    expect(getChatSnapshot()).toMatchObject({
      conversationId: null,
      reservedConversationId: null,
      messages: [],
      activeRunId: null,
      runState: null,
      activity: [],
      reasoning: [],
      roundProse: [],
      reasoningTrimmed: false,
    });
    act(() =>
      applyToolInvocationForTests({ type: "completed", version: 1, invocation_id: "inv-old", truncated: false }),
    );
    expect(getChatSnapshot().activity).toEqual([]);
    expect(getChatSnapshot().conversations.map((row) => row.conversation_id)).toContain("conv-0");
  });

  it("does not send with All when native selection lookup fails", async () => {
    seed();
    vi.mocked(chatMcpSelectionGet).mockRejectedValue(new Error("selection storage unavailable"));
    const { result } = renderHook(() => useChat(true, false));
    await waitFor(() => expect(result.current.mcpSelection.status).toBe("error"));

    await act(async () => result.current.send("must stay blocked"));
    expect(chatMessageSend).not.toHaveBeenCalled();
    expect(getChatSnapshot().messages.map((message) => message.text)).toEqual(["original"]);
  });

  it("reloads a failed selection after the same conversation reopens successfully", async () => {
    seed();
    vi.mocked(chatMcpSelectionGet)
      .mockRejectedValueOnce(new Error("selection storage unavailable"))
      .mockResolvedValueOnce({ version: 1, server_id: "server-recovered" });
    vi.mocked(chatConversationPreview).mockResolvedValue(null);
    vi.mocked(chatConversationOpen).mockResolvedValue(view("conv-0", "Settled", "original"));
    vi.mocked(chatMessageSend).mockResolvedValue({
      version: 1,
      status: "submitted",
      conversation_id: "conv-0" as ConversationId,
      run_id: "run-recovered",
      message: "",
    });
    const { result } = renderHook(() => useChat(true, false));
    await waitFor(() => expect(result.current.mcpSelection.status).toBe("error"));

    await act(async () => result.current.send("blocked until lookup"));
    expect(chatMessageSend).not.toHaveBeenCalled();
    expect(result.current.mcpSelection).toMatchObject({ serverId: null, status: "error" });

    await act(async () => result.current.openConversation("conv-0" as ConversationId));
    await waitFor(() =>
      expect(result.current.mcpSelection).toMatchObject({
        conversationId: "conv-0",
        serverId: "server-recovered",
        status: "ready",
      }),
    );
    await act(async () => result.current.send("now scoped"));
    expect(chatMessageSend).toHaveBeenCalledWith("conv-0", "now scoped", [], "server-recovered");
  });

  it("ignores a late selection lookup after opening another conversation", async () => {
    seed();
    const priorLookup = deferred<{ version: 1; server_id: string | null }>();
    const currentLookup = deferred<{ version: 1; server_id: string | null }>();
    vi.mocked(chatMcpSelectionGet).mockReturnValueOnce(priorLookup.promise).mockReturnValueOnce(currentLookup.promise);
    vi.mocked(chatConversationPreview).mockResolvedValue(null);
    vi.mocked(chatConversationOpen).mockResolvedValue(view("conv-a", "Alpha", "fresh"));
    const { result } = renderHook(() => useChat(true, false));
    await waitFor(() => expect(chatMcpSelectionGet).toHaveBeenCalledTimes(1));

    await act(async () => result.current.openConversation("conv-a" as ConversationId));
    await waitFor(() => expect(chatMcpSelectionGet).toHaveBeenCalledTimes(2));
    await act(async () => currentLookup.resolve({ version: 1, server_id: "server-current" }));
    await act(async () => priorLookup.resolve({ version: 1, server_id: "server-stale" }));

    expect(result.current.mcpSelection).toMatchObject({
      conversationId: "conv-a",
      serverId: "server-current",
      status: "ready",
    });
  });

  it("passes a new chat's selected server to creation and send", async () => {
    vi.mocked(chatConversationCreate).mockResolvedValue(view("conv-new", "New", ""));
    const { result } = renderHook(() => useChat(true, false));
    await act(async () => result.current.selectMcpServer("server-b"));

    await act(async () => result.current.send("scoped question"));
    expect(chatConversationCreate).toHaveBeenCalledWith("scoped question", "server-b");
    expect(chatMessageSend).toHaveBeenCalledWith("conv-new", "scoped question", [], "server-b");
  });

  it("clears a new-chat selection when the account changes without a conversation id", async () => {
    const { result } = renderHook(() => useChat(true, false));
    await act(async () => result.current.selectMcpServer("previous-account-server"));
    expect(result.current.mcpSelection.serverId).toBe("previous-account-server");

    act(() => {
      resetAuthSessionForTests();
      resetChatStore();
    });
    await waitFor(() => expect(result.current.mcpSelection).toMatchObject({ serverId: null, status: "ready" }));
  });

  it("selects immediately, then renders preview before the server settles", async () => {
    seed();
    const preview = deferred<ChatConversationView | null>();
    const open = deferred<ChatConversationView>();
    vi.mocked(chatConversationPreview).mockReturnValue(preview.promise);
    vi.mocked(chatConversationOpen).mockReturnValue(open.promise);
    const { result } = renderHook(() => useChat(true, false));

    let pending!: Promise<void>;
    act(() => {
      pending = result.current.openConversation("conv-a" as ConversationId);
    });
    expect(getChatSnapshot()).toMatchObject({
      conversationId: "conv-a",
      title: "Alpha",
      conversationContentPending: true,
    });
    expect(chatConversationPreview).toHaveBeenCalledWith("conv-a");
    expect(chatConversationOpen).toHaveBeenCalledWith("conv-a");

    await act(async () => preview.resolve(view("conv-a", "Alpha", "cached", "stale")));
    expect(getChatSnapshot().messages.map((message) => message.text)).toEqual(["cached"]);
    expect(getChatSnapshot()).toMatchObject({ availability: "stale", conversationContentPending: false });

    await act(async () => open.resolve(view("conv-a", "Alpha", "fresh")));
    await pending;
    expect(getChatSnapshot().messages.map((message) => message.text)).toEqual(["fresh"]);
    expect(getChatSnapshot()).toMatchObject({ availability: "live", openingConversationId: null });
  });

  it("keeps the latest selection when an older request settles late", async () => {
    seed();
    const previews = [deferred<ChatConversationView | null>(), deferred<ChatConversationView | null>()];
    const opens = [deferred<ChatConversationView>(), deferred<ChatConversationView>()];
    vi.mocked(chatConversationPreview)
      .mockReturnValueOnce(previews[0].promise)
      .mockReturnValueOnce(previews[1].promise);
    vi.mocked(chatConversationOpen).mockReturnValueOnce(opens[0].promise).mockReturnValueOnce(opens[1].promise);
    const { result } = renderHook(() => useChat(true, false));

    act(() => {
      void result.current.openConversation("conv-a" as ConversationId);
      void result.current.openConversation("conv-b" as ConversationId);
    });
    expect(getChatSnapshot()).toMatchObject({ conversationId: "conv-b", title: "Beta" });

    await act(async () => {
      previews[0].resolve(view("conv-a", "Alpha", "late cached", "stale"));
      opens[0].resolve(view("conv-a", "Alpha", "late fresh"));
    });
    expect(getChatSnapshot()).toMatchObject({ conversationId: "conv-b", title: "Beta" });
    expect(getChatSnapshot().messages).toEqual([]);

    await act(async () => previews[1].resolve(null));
    expect(getChatSnapshot().conversationContentPending).toBe(true);
    await act(async () => opens[1].resolve(view("conv-b", "Beta", "winner")));
    expect(getChatSnapshot().messages.map((message) => message.text)).toEqual(["winner"]);
  });

  it("rolls a failed latest selection back to the last settled conversation", async () => {
    seed();
    vi.mocked(chatConversationPreview).mockResolvedValue(null);
    vi.mocked(chatConversationOpen).mockRejectedValue(
      chatRejection("temporarily_unavailable", "The workspace is not responding right now. Try again in a moment."),
    );
    const { result } = renderHook(() => useChat(true, false));

    await act(async () => result.current.openConversation("conv-a" as ConversationId));

    expect(getChatSnapshot()).toMatchObject({
      conversationId: "conv-0",
      title: "Settled",
      conversationContentPending: false,
      error: "The workspace is not responding right now. Try again in a moment.",
    });
    expect(getChatSnapshot().messages.map((message) => message.text)).toEqual(["original"]);
  });

  it("rolls chained opens back to a source that settled authoritatively in the background", async () => {
    seed();
    adoptActiveRun("conv-0", "run-0");
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-0", sequence: 1, text: "partial" });
    const opens = [deferred<ChatConversationView>(), deferred<ChatConversationView>()];
    vi.mocked(chatConversationPreview).mockResolvedValue(null);
    vi.mocked(chatConversationOpen).mockReturnValueOnce(opens[0].promise).mockReturnValueOnce(opens[1].promise);
    const { result } = renderHook(() => useChat(true, false));

    let latest!: Promise<void>;
    act(() => {
      void result.current.openConversation("conv-a" as ConversationId);
      latest = result.current.openConversation("conv-b" as ConversationId);
    });
    expect(getChatSnapshot()).toMatchObject({ conversationId: "conv-b", openingConversationId: "conv-b" });

    await act(async () => {
      applyInferenceForTests({ type: "completed", version: 1, run_id: "run-0", finish_reason: "stop" });
      applyChatProjectionForTests({
        type: "snapshot_replaced",
        version: 1,
        conversation: view("conv-0", "Settled", "partial"),
      });
      opens[1].reject(
        chatRejection("temporarily_unavailable", "The workspace is not responding right now. Try again in a moment."),
      );
      await latest;
    });

    expect(getChatSnapshot()).toMatchObject({
      conversationId: "conv-0",
      activeRunId: null,
      runState: null,
      streamingText: "",
      settlingText: "",
      openingConversationId: null,
      conversationContentPending: false,
    });
    expect(getChatSnapshot().messages.map((message) => message.text)).toEqual(["partial"]);
  });

  it("preserves destination updates through the failed-open hook and the next reopen", async () => {
    seed();
    applyConversationView(view("conv-a", "Alpha", "original"));
    adoptActiveRun("conv-a", "run-a");
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-a", sequence: 1, text: "Before." });
    applyConversationView(view("conv-0", "Settled", "original"));
    const source = getChatSnapshot();
    const firstOpen = deferred<ChatConversationView>();
    const secondOpen = deferred<ChatConversationView>();
    vi.mocked(chatConversationPreview).mockResolvedValue(null);
    vi.mocked(chatConversationOpen).mockReturnValueOnce(firstOpen.promise).mockReturnValueOnce(secondOpen.promise);
    const { result } = renderHook(() => useChat(true, false));

    let pending!: Promise<void>;
    act(() => {
      pending = result.current.openConversation("conv-a" as ConversationId);
      applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-a", sequence: 2, text: " During open." });
      applyInferenceForTests({
        type: "reasoning_summary",
        version: 1,
        run_id: "run-a",
        sequence: 3,
        summary: "Needs approval.",
      });
      applyToolInvocationForTests({
        type: "requested",
        version: 1,
        invocation_id: "inv-a",
        run_id: "run-a",
        tool_id: "project.write_file",
      });
      applyToolInvocationForTests({
        type: "approval_required",
        version: 1,
        invocation_id: "inv-a",
        tool_id: "project.write_file",
      });
    });
    const destination = getChatSnapshot();
    const rejection = chatRejection("temporarily_unavailable", "Could not open this chat.");
    await act(async () => {
      firstOpen.reject(rejection);
      await pending;
    });
    expect(getChatSnapshot()).toEqual({ ...source, error: rejection.message });

    act(() => {
      pending = result.current.openConversation("conv-a" as ConversationId);
    });
    expect(getChatSnapshot()).toEqual(destination);
    const authoritative = { ...view("conv-a", "Alpha", "original"), active_run_id: "run-a" };
    await act(async () => {
      secondOpen.resolve(authoritative);
      await pending;
    });
    expect(getChatSnapshot().activity.map((event) => event.type)).toEqual(["requested", "approval_required"]);
    expect(getChatSnapshot().reasoning).toEqual(destination.reasoning);
    expect(getChatSnapshot().roundProse).toEqual(destination.roundProse);
  });

  it("coalesces repeated clicks while the same conversation is opening", async () => {
    seed();
    const open = deferred<ChatConversationView>();
    vi.mocked(chatConversationPreview).mockResolvedValue(null);
    vi.mocked(chatConversationOpen).mockReturnValue(open.promise);
    const { result } = renderHook(() => useChat(true, false));

    act(() => {
      void result.current.openConversation("conv-a" as ConversationId);
      void result.current.openConversation("conv-a" as ConversationId);
    });

    expect(chatConversationPreview).toHaveBeenCalledTimes(1);
    expect(chatConversationOpen).toHaveBeenCalledTimes(1);
    await act(async () => open.resolve(view("conv-a", "Alpha", "fresh")));
  });

  it("rolls back an authoritative not-found before refreshing the list", async () => {
    seed();
    vi.mocked(chatConversationPreview).mockResolvedValue(null);
    vi.mocked(chatConversationOpen).mockRejectedValue(
      chatRejection("not_found", "That conversation no longer exists."),
    );
    const { result } = renderHook(() => useChat(true, false));

    await act(async () => result.current.openConversation("conv-a" as ConversationId));

    expect(getChatSnapshot()).toMatchObject({
      conversationId: "conv-0",
      title: "Settled",
      conversationContentPending: false,
      error: null,
    });
    expect(getChatSnapshot().messages.map((message) => message.text)).toEqual(["original"]);
  });
});

describe("a send refreshes the sidebar only after it settles", () => {
  const created = { ...view("conv-new", "New conversation", ""), messages: [] };
  const workspace: ChatWorkspaceBootstrap = {
    version: 1,
    conversations: [
      {
        version: 1,
        conversation_id: "conv-new" as ConversationId,
        title: "New conversation",
        updated_at: "2026-10-01T00:00:00Z",
      },
      { version: 1, conversation_id: "conv-0" as ConversationId, title: "Settled", updated_at: "2026-09-01T00:00:00Z" },
    ],
  };
  const accepted: ChatSubmitResult = {
    version: 1,
    status: "submitted",
    conversation_id: "conv-new" as ConversationId,
    run_id: "run-1",
    message: "",
  };

  beforeEach(() => {
    vi.stubGlobal("__TAURI_INTERNALS__", {});
    vi.mocked(chatWorkspaceBootstrap).mockResolvedValue(workspace);
    vi.mocked(listen).mockResolvedValue(() => {});
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    resetComposerDraft();
  });

  function sendFromNewChat(prompt: string) {
    vi.mocked(chatConversationCreate).mockResolvedValue(created);
    const { result } = renderHook(() => useChat(true, false));
    let pending!: Promise<unknown>;
    act(() => {
      pending = result.current.send(prompt);
    });
    return { pending };
  }

  it("sends a new chat before the workspace bootstrap is requested", async () => {
    const send = deferred<ChatSubmitResult>();
    vi.mocked(chatMessageSend).mockReturnValue(send.promise);
    const { pending } = sendFromNewChat("hello");

    await waitFor(() => expect(chatMessageSend).toHaveBeenCalledWith("conv-new", "hello", [], null));
    expect(chatWorkspaceBootstrap).not.toHaveBeenCalled();

    await act(async () => {
      send.resolve(accepted);
      await pending;
    });
    expect(vi.mocked(chatWorkspaceBootstrap).mock.invocationCallOrder[0]).toBeGreaterThan(
      vi.mocked(chatMessageSend).mock.invocationCallOrder[0],
    );
  });

  it("refreshes once after an accepted send and the sidebar takes the new row from it", async () => {
    const send = deferred<ChatSubmitResult>();
    vi.mocked(chatMessageSend).mockReturnValue(send.promise);
    const { pending } = sendFromNewChat("hello");
    await waitFor(() => expect(chatMessageSend).toHaveBeenCalledTimes(1));
    expect(chatWorkspaceBootstrap).not.toHaveBeenCalled();

    await act(async () => {
      send.resolve(accepted);
      await pending;
    });

    await waitFor(() =>
      expect(getChatSnapshot().conversations.map((row) => row.conversation_id)).toEqual(["conv-new", "conv-0"]),
    );
    expect(chatWorkspaceBootstrap).toHaveBeenCalledTimes(1);
    expect(getChatSnapshot().error).toBeNull();
  });

  it("refreshes once after a failed result and keeps the rollback and the error text", async () => {
    const send = deferred<ChatSubmitResult>();
    vi.mocked(chatMessageSend).mockReturnValue(send.promise);
    const { pending } = sendFromNewChat("hello");
    await waitFor(() => expect(chatMessageSend).toHaveBeenCalledTimes(1));
    expect(chatWorkspaceBootstrap).not.toHaveBeenCalled();

    await act(async () => {
      send.resolve({
        ...accepted,
        status: "failed",
        run_id: null,
        message: "Model is unavailable.",
      });
      await pending;
    });

    await waitFor(() => expect(chatWorkspaceBootstrap).toHaveBeenCalledTimes(1));
    await waitFor(() =>
      expect(getChatSnapshot().conversations.map((row) => row.conversation_id)).toEqual(["conv-new", "conv-0"]),
    );
    expect(getChatSnapshot().error).toBe("Model is unavailable.");
    expect(getChatSnapshot().messages.map((message) => message.text)).not.toContain("hello");
    expect(getComposerDraft("conv-new")).toBe("hello");
  });

  it("refreshes once after a rejected send and keeps the error text", async () => {
    const send = deferred<ChatSubmitResult>();
    vi.mocked(chatMessageSend).mockReturnValue(send.promise);
    const { pending } = sendFromNewChat("hello");
    await waitFor(() => expect(chatMessageSend).toHaveBeenCalledTimes(1));
    expect(chatWorkspaceBootstrap).not.toHaveBeenCalled();

    await act(async () => {
      send.reject(new Error("ipc closed"));
      await pending;
    });

    await waitFor(() => expect(chatWorkspaceBootstrap).toHaveBeenCalledTimes(1));
    await waitFor(() =>
      expect(getChatSnapshot().conversations.map((row) => row.conversation_id)).toEqual(["conv-new", "conv-0"]),
    );
    expect(getChatSnapshot().error).toBe("Could not send the message. The secure runtime did not respond.");
    expect(getChatSnapshot().messages.map((message) => message.text)).not.toContain("hello");
    expect(getComposerDraft("conv-new")).toBe("hello");
  });

  it("does not refresh from a send into a chat that already exists", async () => {
    seed();
    vi.mocked(chatMessageSend).mockResolvedValue({ ...accepted, conversation_id: "conv-0" as ConversationId });
    vi.mocked(chatConversationPreview).mockResolvedValue(null);
    vi.mocked(chatConversationOpen).mockResolvedValue(view("conv-0", "Settled", "original"));
    const { result } = renderHook(() => useChat(true, false));
    await act(async () => result.current.openConversation("conv-0" as ConversationId));
    const bootstrapCalls = vi.mocked(chatWorkspaceBootstrap).mock.calls.length;

    await act(async () => {
      await result.current.send("follow up");
    });

    expect(chatMessageSend).toHaveBeenCalledWith("conv-0", "follow up", [], null);
    expect(vi.mocked(chatWorkspaceBootstrap).mock.calls.length).toBe(bootstrapCalls);
  });

  it("keeps a run error that lands while the post-send refresh is pending, when the refresh succeeds", async () => {
    const refresh = deferred<typeof workspace>();
    vi.mocked(chatWorkspaceBootstrap).mockReturnValue(refresh.promise);
    vi.mocked(chatMessageSend).mockResolvedValue(accepted);
    const { pending } = sendFromNewChat("hello");
    await waitFor(() => expect(chatWorkspaceBootstrap).toHaveBeenCalledTimes(1));

    await act(async () => {
      await pending;
    });
    act(() => setChatError("Run failed"));
    await act(async () => {
      refresh.resolve(workspace);
      await refresh.promise;
    });

    await waitFor(() =>
      expect(getChatSnapshot().conversations.map((row) => row.conversation_id)).toEqual(["conv-new", "conv-0"]),
    );
    expect(getChatSnapshot().error).toBe("Run failed");
  });

  it("keeps a run error that lands while the post-send refresh is pending, when the refresh fails", async () => {
    const refresh = deferred<typeof workspace>();
    vi.mocked(chatWorkspaceBootstrap).mockReturnValue(refresh.promise);
    vi.mocked(chatMessageSend).mockResolvedValue(accepted);
    const { pending } = sendFromNewChat("hello");
    await waitFor(() => expect(chatWorkspaceBootstrap).toHaveBeenCalledTimes(1));

    await act(async () => {
      await pending;
    });
    act(() => setChatError("Run failed"));
    await act(async () => {
      refresh.reject(new Error("ipc closed"));
      await refresh.promise.catch(() => {});
    });

    expect(getChatSnapshot().error).toBe("Run failed");
  });

  it("shows the refresh failure when a send that succeeded left no error on screen", async () => {
    vi.mocked(chatMessageSend).mockResolvedValue(accepted);
    vi.mocked(chatWorkspaceBootstrap).mockRejectedValue(new Error("ipc closed"));
    const { pending } = sendFromNewChat("hello");

    await act(async () => {
      await pending;
    });

    await waitFor(() => expect(getChatSnapshot().error).toBe("Could not load conversations."));
  });
});
