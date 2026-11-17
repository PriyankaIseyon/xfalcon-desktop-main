import { describe, expect, it } from "vitest";
import type {
  ChatConversationSummary,
  ChatConversationView,
  ConversationId,
  RunId,
} from "../../lib/generated/contracts";
import {
  addOptimisticUserMessage,
  adoptActiveRun,
  applyChatProjectionForTests,
  applyConversationView,
  applyConversationPreview,
  applyCreatedConversationView,
  applyInferenceForTests,
  applyRunEventForTests,
  applyToolInvocationForTests,
  beginNewConversation,
  beginConversationOpen,
  beginRunSubmission,
  beginTurnRevision,
  finishTurnRevision,
  dropOptimisticUserMessage,
  finishRunSubmission,
  getChatSnapshot,
  resetChatStore,
  removeConversationFromStore,
  rollbackConversationOpen,
  completeConversationOpen,
  setChatStoreForTests,
  setReservedConversation,
  setWorkspaceBootstrap,
  subscribeChat,
} from "./chatStore";
import { getComposerDraft, setComposerDraft } from "./composerStore";
import { reduceInferenceEvent } from "./chatReduce";
import type { ChatStoreSnapshot } from "./chatStore";

function conversationView(
  conversationId: string,
  messageSpecs: Array<{ id: string; role: "user" | "assistant"; text: string; sequence: number }>,
  activeRunId: string | null = null,
  title = "Test conversation",
): ChatConversationView {
  return {
    version: 1,
    conversation_id: conversationId,
    title,
    availability: "live",
    messages: messageSpecs.map((message) => ({
      version: 1,
      message_id: message.id,
      role: message.role,
      text: message.text,
      sequence: BigInt(message.sequence),
      tool_calls: [],
      attachments: [],
    })),
    active_run_id: activeRunId,
  };
}

function summary(conversationId: string, title: string): ChatConversationSummary {
  return {
    version: 1,
    conversation_id: conversationId,
    title,
    updated_at: "2026-08-21T00:00:00Z",
  };
}

const base: ChatStoreSnapshot = {
  conversationId: "conv-1",
  reservedConversationId: null,
  title: "t",
  availability: null,
  messages: [],
  streamingText: "",
  settlingText: "",
  compactingRunId: null,
  // Inference events are admitted only for the bound run.
  activeRunId: "run-1",
  runState: null,
  error: null,
  conversations: [],
  workspaceLoaded: false,
  openingConversationId: null,
  conversationContentPending: false,
  recoveryNotice: null,
  activity: [],
  roundProse: [],
  reasoning: [],
  reasoningTrimmed: false,
  revising: false,
  awaitingRun: false,
};

describe("chatStore", () => {
  it("keeps a conversation reserved for attachments out of the sidebar until it is released", () => {
    resetChatStore();
    const rows = [summary("conv-reserved", "New Chat"), summary("conv-other", "Q3 review")];
    setWorkspaceBootstrap(rows);
    setReservedConversation("conv-reserved");
    expect(getChatSnapshot().conversations.map((row) => row.conversation_id)).toEqual(["conv-other"]);
    // A later list refresh does not bring it back while it is reserved.
    setWorkspaceBootstrap(rows);
    expect(getChatSnapshot().conversations.map((row) => row.conversation_id)).toEqual(["conv-other"]);
    setReservedConversation(null);
    setWorkspaceBootstrap(rows);
    expect(getChatSnapshot().conversations).toHaveLength(2);
  });

  it("notifies subscribers on reset", () => {
    resetChatStore();
    let count = 0;
    const unsubscribe = subscribeChat(() => {
      count += 1;
    });
    resetChatStore();
    unsubscribe();
    expect(count).toBeGreaterThan(0);
    expect(getChatSnapshot().messages).toHaveLength(0);
  });

  it("reducer contract matches store inference handling", () => {
    const patch = reduceInferenceEvent(base, {
      type: "text_delta",
      version: 1,
      run_id: "run-1",
      sequence: 1,
      text: "x",
    });
    expect(patch.streamingText).toBe("x");
  });

  it("preserves composer draft when starting a new conversation", () => {
    resetChatStore();
    setComposerDraft("draft text");
    beginNewConversation();
    // The draft lives in its own store now, so "New chat" cannot disturb it at
    // all — but the guarantee is the same one this test always asserted.
    expect(getComposerDraft()).toBe("draft text");
    expect(getChatSnapshot().conversationId).toBeNull();
  });

  it("clears the composer draft when the store is reset", () => {
    setComposerDraft("half a sentence");
    resetChatStore();
    expect(getComposerDraft()).toBe("");
  });

  it("does not notify transcript subscribers when the draft changes", () => {
    resetChatStore();
    let notifications = 0;
    const unsubscribe = subscribeChat(() => {
      notifications += 1;
    });

    setComposerDraft("typing");
    setComposerDraft("typing more");

    // The whole point of the split: a keystroke is not a transcript change, so
    // nothing that renders the transcript may wake for one.
    expect(notifications).toBe(0);
    unsubscribe();
  });

  it("ignores projection run_state_changed for a conversation that is not on screen", () => {
    resetChatStore();
    applyConversationView(
      conversationView("conv-1", [{ id: "m-1", role: "user", text: "hi", sequence: 1 }], "run-active"),
    );
    applyChatProjectionForTests({
      type: "run_state_changed",
      version: 1,
      conversation_id: "conv-other",
      run_id: "run-foreign",
      state: "running",
    });
    const snapshot = getChatSnapshot();
    expect(snapshot.activeRunId).toBe("run-active");
    expect(snapshot.runState).toBeNull();
  });

  it("adopts the run a projection run_state_changed announces for this conversation", () => {
    resetChatStore();
    applyConversationView(conversationView("conv-1", [{ id: "m-1", role: "user", text: "hi", sequence: 1 }], null));
    applyChatProjectionForTests({
      type: "run_state_changed",
      version: 1,
      conversation_id: "conv-1",
      run_id: "run-new",
      state: "queued",
    });
    const snapshot = getChatSnapshot();
    expect(snapshot.activeRunId).toBe("run-new");
    expect(snapshot.runState).toBe("queued");
  });

  it("retires a stale active run when Stop reconciles a terminal snapshot", () => {
    resetChatStore();
    applyConversationView(
      conversationView("conv-1", [{ id: "m-1", role: "user", text: "hi", sequence: 1 }], "run-active"),
    );
    applyChatProjectionForTests({
      type: "run_state_changed",
      version: 1,
      conversation_id: "conv-1",
      run_id: "run-active",
      state: "running",
    });
    applyInferenceForTests({
      type: "text_delta",
      version: 1,
      run_id: "run-active",
      sequence: 1,
      text: "partial",
    });

    applyConversationView(
      conversationView(
        "conv-1",
        [
          { id: "m-1", role: "user", text: "hi", sequence: 1 },
          { id: "m-2", role: "assistant", text: "settled", sequence: 2 },
        ],
        null,
      ),
    );

    expect(getChatSnapshot()).toMatchObject({
      activeRunId: null,
      runState: null,
      streamingText: "",
      settlingText: "",
      activity: [],
      roundProse: [],
    });
  });

  it("keeps a failed run as the retry target after terminal reconciliation", () => {
    resetChatStore();
    applyConversationView(conversationView("conv-1", [], "run-failed"));
    applyChatProjectionForTests({
      type: "run_state_changed",
      version: 1,
      conversation_id: "conv-1",
      run_id: "run-failed",
      state: "failed",
    });

    applyConversationView(conversationView("conv-1", [], null));

    expect(getChatSnapshot()).toMatchObject({
      activeRunId: "run-failed",
      runState: "failed",
      recoveryNotice: expect.stringContaining("interrupted"),
    });
  });

  it("keeps a revision claimed through its own conversation's snapshot, until the run is adopted", () => {
    // Regenerate and edit cut the transcript, and the backend then emits a
    // snapshot of the cut conversation BEFORE the command returns its run id.
    // That snapshot used to clear `revising`, leaving the turn with no run, no
    // submission and no revision: the thinking pill vanished, then came back
    // when the run was adopted.
    resetChatStore();
    applyConversationView(
      conversationView("conv-1", [
        { id: "u-1", role: "user", text: "hi", sequence: 1 },
        { id: "a-1", role: "assistant", text: "hello", sequence: 2 },
      ]),
    );
    expect(beginTurnRevision("a-1")).not.toBeNull();
    applyChatProjectionForTests({
      type: "snapshot_replaced",
      version: 1,
      conversation: conversationView("conv-1", [{ id: "u-1", role: "user", text: "hi", sequence: 1 }]),
    });
    expect(getChatSnapshot().revising).toBe(true);
    expect(getChatSnapshot().messages.map((message) => message.id)).toEqual(["u-1"]);

    adoptActiveRun("conv-1", "run-2");
    finishTurnRevision("conv-1");
    const settled = getChatSnapshot();
    expect(settled.revising).toBe(false);
    expect(settled.activeRunId).toBe("run-2");
  });

  it("drops a revision when another conversation replaces the open one", () => {
    resetChatStore();
    applyConversationView(
      conversationView("conv-1", [
        { id: "u-1", role: "user", text: "hi", sequence: 1 },
        { id: "a-1", role: "assistant", text: "hello", sequence: 2 },
      ]),
    );
    beginTurnRevision("a-1");
    applyConversationView(conversationView("conv-2", [{ id: "u-2", role: "user", text: "yo", sequence: 1 }]));
    expect(getChatSnapshot().revising).toBe(false);
  });

  it("ignores regressive snapshot_replaced projections for the same conversation", () => {
    resetChatStore();
    applyConversationView(
      conversationView(
        "conv-1",
        [
          { id: "m-1", role: "user", text: "one", sequence: 1 },
          { id: "m-2", role: "assistant", text: "two", sequence: 2 },
        ],
        null,
      ),
    );
    applyChatProjectionForTests({
      type: "snapshot_replaced",
      version: 1,
      conversation: conversationView("conv-1", [{ id: "m-1", role: "user", text: "one", sequence: 1 }]),
    });
    expect(getChatSnapshot().messages).toHaveLength(2);

    applyChatProjectionForTests({
      type: "snapshot_replaced",
      version: 1,
      conversation: conversationView(
        "conv-1",
        [
          { id: "m-1", role: "user", text: "one", sequence: 1 },
          { id: "m-2", role: "assistant", text: "two", sequence: 2 },
          { id: "m-3", role: "user", text: "three", sequence: 3 },
        ],
        null,
      ),
    });
    expect(getChatSnapshot().messages).toHaveLength(3);
  });

  it("ignores snapshot_replaced for a conversation that is not on screen", () => {
    // A background run finalizing elsewhere emits the same broadcast;
    // adopting it would teleport the user into that conversation.
    resetChatStore();
    applyConversationView(conversationView("conv-1", [{ id: "m-1", role: "user", text: "mine", sequence: 1 }]));
    applyChatProjectionForTests({
      type: "snapshot_replaced",
      version: 1,
      conversation: conversationView("conv-other", [
        { id: "m-x", role: "assistant", text: "someone else's answer", sequence: 1 },
      ]),
    });
    const snapshot = getChatSnapshot();
    expect(snapshot.conversationId).toBe("conv-1");
    expect(snapshot.messages.map((message) => message.text)).toEqual(["mine"]);
  });

  it("applies a model title upgrade to the open chat and the recents list", () => {
    resetChatStore();
    setWorkspaceBootstrap([summary("conv-1", "check what tables do i have in…"), summary("conv-2", "Other chat")]);
    applyConversationView(
      conversationView(
        "conv-1",
        [{ id: "m-1", role: "user", text: "check what tables do i have in mcp", sequence: 1 }],
        null,
        "check what tables do i have in…",
      ),
    );
    applyChatProjectionForTests({
      type: "snapshot_replaced",
      version: 1,
      conversation: conversationView(
        "conv-1",
        [{ id: "m-1", role: "user", text: "check what tables do i have in mcp", sequence: 1 }],
        null,
        "List MCP database tables",
      ),
    });
    const snapshot = getChatSnapshot();
    expect(snapshot.title).toBe("List MCP database tables");
    expect(snapshot.conversations.map((row) => row.title)).toEqual(["List MCP database tables", "Other chat"]);
  });

  it("honors a model title on a recents row without opening that conversation", () => {
    resetChatStore();
    setWorkspaceBootstrap([summary("conv-1", "Open chat"), summary("conv-2", "check what tables do i have in…")]);
    applyConversationView(
      conversationView("conv-1", [{ id: "m-1", role: "user", text: "mine", sequence: 1 }], null, "Open chat"),
    );
    applyChatProjectionForTests({
      type: "snapshot_replaced",
      version: 1,
      conversation: conversationView(
        "conv-2",
        [{ id: "m-x", role: "user", text: "check what tables do i have in mcp", sequence: 1 }],
        null,
        "List MCP database tables",
      ),
    });
    const snapshot = getChatSnapshot();
    expect(snapshot.conversationId).toBe("conv-1");
    expect(snapshot.title).toBe("Open chat");
    expect(snapshot.messages.map((message) => message.text)).toEqual(["mine"]);
    expect(snapshot.conversations.map((row) => row.title)).toEqual(["Open chat", "List MCP database tables"]);
  });

  it("leaves recents unchanged when a background snapshot repeats the listed title", () => {
    resetChatStore();
    setWorkspaceBootstrap([summary("conv-1", "Open chat"), summary("conv-2", "List MCP database tables")]);
    applyConversationView(
      conversationView("conv-1", [{ id: "m-1", role: "user", text: "mine", sequence: 1 }], null, "Open chat"),
    );
    const before = getChatSnapshot().conversations;
    applyChatProjectionForTests({
      type: "snapshot_replaced",
      version: 1,
      conversation: conversationView(
        "conv-2",
        [{ id: "m-x", role: "user", text: "check what tables do i have in mcp", sequence: 1 }],
        null,
        "List MCP database tables",
      ),
    });
    expect(getChatSnapshot().conversations).toBe(before);
  });

  it("loads the workspace without attaching a historical run notice to new chat", () => {
    resetChatStore();
    setWorkspaceBootstrap([]);
    expect(getChatSnapshot().recoveryNotice).toBeNull();
    expect(getChatSnapshot().workspaceLoaded).toBe(true);
  });

  it("applies the startup recovery notice without replacing it on a later workspace refresh", () => {
    resetChatStore();
    setWorkspaceBootstrap([], "The previous response was interrupted.");
    expect(getChatSnapshot().recoveryNotice).toBe("The previous response was interrupted.");

    setWorkspaceBootstrap([]);
    expect(getChatSnapshot().recoveryNotice).toBe("The previous response was interrupted.");
  });

  it("maps stale availability from conversation views", () => {
    resetChatStore();
    applyConversationView({
      ...conversationView("conv-offline", [{ id: "m-1", role: "user", text: "cached", sequence: 1 }]),
      availability: "stale",
    });
    expect(getChatSnapshot().availability).toBe("stale");
  });

  it("selects immediately, applies a stale preview, and completes with server truth", () => {
    resetChatStore();
    setWorkspaceBootstrap([summary("conv-1", "First"), summary("conv-2", "Second")]);
    applyConversationView(conversationView("conv-1", [{ id: "m-1", role: "user", text: "old", sequence: 1 }]));

    const restore = beginConversationOpen("conv-2");
    expect(restore).not.toBeNull();
    expect(getChatSnapshot()).toMatchObject({
      conversationId: "conv-2",
      title: "Second",
      messages: [],
      openingConversationId: "conv-2",
      conversationContentPending: true,
    });

    applyConversationPreview(
      {
        ...conversationView("conv-2", [{ id: "m-2", role: "user", text: "cached", sequence: 1 }]),
        availability: "stale",
      },
      "conv-2",
    );
    expect(getChatSnapshot()).toMatchObject({
      availability: "stale",
      conversationContentPending: false,
      openingConversationId: "conv-2",
    });

    completeConversationOpen(
      conversationView("conv-2", [{ id: "m-3", role: "assistant", text: "fresh", sequence: 2 }]),
      "conv-2",
    );
    expect(getChatSnapshot()).toMatchObject({
      availability: "live",
      openingConversationId: null,
      conversationContentPending: false,
    });
    expect(getChatSnapshot().messages.map((message) => message.text)).toEqual(["fresh"]);
  });

  it("restores the complete snapshot and admitted invocation set after a failed open", () => {
    resetChatStore();
    setWorkspaceBootstrap([summary("conv-1", "First"), summary("conv-2", "Second")]);
    applyConversationView(conversationView("conv-1", [{ id: "m-1", role: "user", text: "old", sequence: 1 }], "run-1"));
    applyChatProjectionForTests({
      type: "run_state_changed",
      version: 1,
      conversation_id: "conv-1",
      run_id: "run-1",
      state: "interrupted",
    });
    applyToolInvocationForTests({
      type: "requested",
      version: 1,
      run_id: "run-1",
      invocation_id: "inv-1",
      tool_id: "lookup",
    });
    const before = getChatSnapshot();

    const restore = beginConversationOpen("conv-2");
    rollbackConversationOpen(restore);
    expect(getChatSnapshot()).toEqual(before);

    applyToolInvocationForTests({
      type: "completed",
      version: 1,
      invocation_id: "inv-1",
      truncated: false,
    });
    expect(getChatSnapshot().activity).toHaveLength(2);
  });

  it("does not let a late local preview replace an authoritative projection event", () => {
    resetChatStore();
    setWorkspaceBootstrap([summary("conv-2", "Second")]);
    beginConversationOpen("conv-2");
    applyChatProjectionForTests({
      type: "snapshot_replaced",
      version: 1,
      conversation: conversationView(
        "conv-2",
        [{ id: "fresh", role: "assistant", text: "server", sequence: 2 }],
        null,
        "Second",
      ),
    });
    applyConversationPreview(
      {
        ...conversationView("conv-2", [{ id: "stale", role: "assistant", text: "cache", sequence: 1 }]),
        availability: "stale",
      },
      "conv-2",
    );

    expect(getChatSnapshot()).toMatchObject({ availability: "live", openingConversationId: null });
    expect(getChatSnapshot().messages.map((message) => message.text)).toEqual(["server"]);
  });

  it("keeps recovery scoped to its conversation across workspace refresh and navigation", () => {
    resetChatStore();
    applyConversationView(conversationView("conv-1", [], "run-1"));
    applyChatProjectionForTests({
      type: "run_state_changed",
      version: 1,
      conversation_id: "conv-1",
      run_id: "run-1",
      state: "interrupted",
    });
    expect(getChatSnapshot().recoveryNotice).toContain("interrupted");
    setWorkspaceBootstrap([]);
    expect(getChatSnapshot().recoveryNotice).toContain("interrupted");
    applyChatProjectionForTests({
      type: "run_state_changed",
      version: 1,
      conversation_id: "other",
      run_id: "other-run",
      state: "running",
    });
    expect(getChatSnapshot().recoveryNotice).toContain("interrupted");
    applyConversationView(conversationView("conv-2", [], null));
    expect(getChatSnapshot().recoveryNotice).toBeNull();
  });

  it("clears an old recovery notice when the conversation announces a replacement run", () => {
    resetChatStore();
    applyConversationView(conversationView("conv-1", [], "run-1"));
    applyChatProjectionForTests({
      type: "run_state_changed",
      version: 1,
      conversation_id: "conv-1",
      run_id: "run-1",
      state: "interrupted",
    });
    expect(getChatSnapshot().recoveryNotice).toContain("interrupted");
    applyChatProjectionForTests({
      type: "run_state_changed",
      version: 1,
      conversation_id: "conv-1",
      run_id: "run-2",
      state: "running",
    });
    expect(getChatSnapshot()).toMatchObject({ activeRunId: "run-2", runState: "running", recoveryNotice: null });
  });
});

describe("chatStore live conversation continuity", () => {
  function bindFirstRun() {
    resetChatStore();
    setWorkspaceBootstrap([summary("conv-1", "First"), summary("conv-2", "Second")]);
    applyConversationView(
      conversationView("conv-1", [{ id: "m-1", role: "user", text: "question", sequence: 1 }], "run-1"),
    );
    applyChatProjectionForTests({
      type: "run_state_changed",
      version: 1,
      conversation_id: "conv-1",
      run_id: "run-1",
      state: "running",
    });
  }

  function openSecondConversation() {
    beginConversationOpen("conv-2");
    completeConversationOpen(conversationView("conv-2", []), "conv-2");
  }

  it("keeps an adopted run streaming in the background and restores every live surface on return", () => {
    bindFirstRun();
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 1, text: "I will check." });
    applyInferenceForTests({
      type: "reasoning_summary",
      version: 1,
      run_id: "run-1",
      sequence: 2,
      summary: "Find the source.",
    });
    applyToolInvocationForTests({
      type: "requested",
      version: 1,
      invocation_id: "inv-1",
      run_id: "run-1",
      tool_id: "project.read_file",
    });

    openSecondConversation();
    adoptActiveRun("conv-2", "run-2");
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-2", sequence: 1, text: "Second chat." });
    applyToolInvocationForTests({ type: "started", version: 1, invocation_id: "inv-1" });
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 3, text: "The result." });
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 3, text: "duplicate" });
    applyInferenceForTests({
      type: "reasoning_summary",
      version: 1,
      run_id: "run-1",
      sequence: 4,
      summary: "Use the result.",
    });
    expect(getChatSnapshot().streamingText).toBe("Second chat.");

    beginConversationOpen("conv-1");
    expect(getChatSnapshot()).toMatchObject({
      conversationId: "conv-1",
      activeRunId: "run-1",
      runState: "running",
      streamingText: "The result.",
      roundProse: [{ text: "I will check.", afterCallCount: 0 }],
      reasoning: [
        { text: "Find the source.", afterCallCount: 0 },
        { text: "Use the result.", afterCallCount: 1 },
      ],
    });
    expect(getChatSnapshot().activity.map((event) => event.type)).toEqual(["requested", "started"]);

    applyConversationPreview({ ...conversationView("conv-1", [], null), availability: "stale" }, "conv-1");
    expect(getChatSnapshot().streamingText).toBe("The result.");
    completeConversationOpen(conversationView("conv-1", [], "run-1"), "conv-1");
    expect(getChatSnapshot().streamingText).toBe("The result.");
    expect(getChatSnapshot().activity.map((event) => event.type)).toEqual(["requested", "started"]);
  });

  it("hands a background terminal run to its durable snapshot and rejects late events", () => {
    bindFirstRun();
    applyInferenceForTests({
      type: "reasoning_summary",
      version: 1,
      run_id: "run-1",
      sequence: 1,
      summary: "Verified the answer.",
    });
    applyToolInvocationForTests({
      type: "requested",
      version: 1,
      invocation_id: "inv-1",
      run_id: "run-1",
      tool_id: "project.read_file",
    });
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 2, text: "Done." });
    openSecondConversation();

    applyInferenceForTests({ type: "completed", version: 1, run_id: "run-1", finish_reason: "stop" });
    applyRunEventForTests({
      type: "state_changed",
      version: 1,
      run_id: "run-1",
      state: "completed",
    });
    const durable = conversationView(
      "conv-1",
      [
        { id: "m-1", role: "user", text: "question", sequence: 1 },
        { id: "m-2", role: "assistant", text: "Done.", sequence: 2 },
      ],
      null,
    );
    applyChatProjectionForTests({ type: "snapshot_replaced", version: 1, conversation: durable });

    applyToolInvocationForTests({ type: "completed", version: 1, invocation_id: "inv-1", truncated: false });
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 3, text: "late" });
    beginConversationOpen("conv-1");
    completeConversationOpen(durable, "conv-1");

    expect(getChatSnapshot()).toMatchObject({
      activeRunId: null,
      streamingText: "",
      settlingText: "",
      activity: [],
      roundProse: [],
      reasoning: [{ text: "Verified the answer.", afterCallCount: 0 }],
    });
    expect(getChatSnapshot().messages.map((message) => message.text)).toEqual(["question", "Done."]);
  });

  it("preserves reasoning when native open emits the durable snapshot before duplicate completion", () => {
    bindFirstRun();
    applyInferenceForTests({
      type: "reasoning_summary",
      version: 1,
      run_id: "run-1",
      sequence: 1,
      summary: "Reasoning survives.",
    });
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 2, text: "Done." });
    applyInferenceForTests({ type: "completed", version: 1, run_id: "run-1", finish_reason: "stop" });
    const settling = getChatSnapshot();

    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 3, text: "late" });
    expect(getChatSnapshot()).toEqual(settling);

    const durable = conversationView(
      "conv-1",
      [
        { id: "m-1", role: "user", text: "question", sequence: 1 },
        { id: "m-2", role: "assistant", text: "Done.", sequence: 2 },
      ],
      null,
    );
    applyChatProjectionForTests({ type: "snapshot_replaced", version: 1, conversation: durable });
    const afterSnapshot = getChatSnapshot();
    const completed = {
      type: "run_state_changed",
      version: 1,
      conversation_id: "conv-1",
      run_id: "run-1",
      state: "completed",
    } as const;
    applyChatProjectionForTests(completed);
    applyChatProjectionForTests(completed);

    expect(getChatSnapshot()).toEqual(afterSnapshot);
    expect(getChatSnapshot()).toMatchObject({
      activeRunId: null,
      streamingText: "",
      settlingText: "",
      reasoning: [{ text: "Reasoning survives.", afterCallCount: 0 }],
    });
  });

  it("does not hand a revision in one conversation to another that has retained live state", () => {
    // Switching to a conversation with retained reasoning reads the destination
    // from a snapshot built on the CURRENT state. That snapshot used to carry
    // `revising`, so the destination opened locked: edit and regenerate disabled,
    // and nothing in it could ever finish the revision.
    bindFirstRun();
    applyInferenceForTests({
      type: "reasoning_summary",
      version: 1,
      run_id: "run-1",
      sequence: 1,
      summary: "Kept for conv-1.",
    });
    applyConversationView(
      conversationView("conv-2", [
        { id: "u-2", role: "user", text: "yo", sequence: 1 },
        { id: "a-2", role: "assistant", text: "hey", sequence: 2 },
      ]),
    );
    expect(beginTurnRevision("a-2")).not.toBeNull();
    expect(getChatSnapshot().revising).toBe(true);

    applyConversationView(
      conversationView("conv-1", [
        { id: "m-1", role: "user", text: "question", sequence: 1 },
        { id: "m-2", role: "assistant", text: "answer", sequence: 2 },
      ]),
    );

    expect(getChatSnapshot().conversationId).toBe("conv-1");
    // The retained path was taken: conv-1's reasoning came back with it.
    expect(getChatSnapshot().reasoning).toEqual([{ text: "Kept for conv-1.", afterCallCount: 0 }]);
    expect(getChatSnapshot().revising).toBe(false);
  });

  it("rolls a failed open back to the latest retained background state", () => {
    bindFirstRun();
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 1, text: "one" });
    applyToolInvocationForTests({
      type: "requested",
      version: 1,
      invocation_id: "inv-1",
      run_id: "run-1",
      tool_id: "project.read_file",
    });
    const restore = beginConversationOpen("conv-2");
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 2, text: "two" });
    applyToolInvocationForTests({ type: "started", version: 1, invocation_id: "inv-1" });

    rollbackConversationOpen(restore);

    expect(getChatSnapshot().streamingText).toBe("two");
    expect(getChatSnapshot().roundProse).toEqual([{ text: "one", afterCallCount: 0 }]);
    expect(getChatSnapshot().activity.map((event) => event.type)).toEqual(["requested", "started"]);
  });

  it("retains destination progress and approval when a pending reopen rolls back", () => {
    bindFirstRun();
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 1, text: "Before." });
    openSecondConversation();
    const source = getChatSnapshot();
    const restore = beginConversationOpen("conv-1");
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 2, text: " During open." });
    applyInferenceForTests({
      type: "reasoning_summary",
      version: 1,
      run_id: "run-1",
      sequence: 3,
      summary: "Needs approval.",
    });
    applyToolInvocationForTests({
      type: "requested",
      version: 1,
      invocation_id: "inv-pending-open",
      run_id: "run-1",
      tool_id: "project.write_file",
    });
    applyToolInvocationForTests({
      type: "approval_required",
      version: 1,
      invocation_id: "inv-pending-open",
      tool_id: "project.write_file",
    });
    const destination = getChatSnapshot();

    rollbackConversationOpen(restore);
    expect(getChatSnapshot()).toEqual(source);

    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 4, text: "After rollback." });
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 2, text: "duplicate" });
    beginConversationOpen("conv-1");
    expect(getChatSnapshot()).toEqual({ ...destination, streamingText: "After rollback." });

    applyToolInvocationForTests({ type: "approved", version: 1, invocation_id: "inv-pending-open" });
    expect(getChatSnapshot().activity.map((event) => event.type)).toEqual([
      "requested",
      "approval_required",
      "approved",
    ]);
  });

  it.each(["failed", "interrupted"] as const)(
    "preserves a %s retry target when a background snapshot precedes open rollback",
    (runState) => {
      bindFirstRun();
      applyInferenceForTests({
        type: "reasoning_summary",
        version: 1,
        run_id: "run-1",
        sequence: 1,
        summary: "Kept thinking.",
      });
      applyInferenceForTests({
        type: "failed",
        version: 1,
        run_id: "run-1",
        code: "provider_unavailable",
        message: "Provider disconnected.",
        retryable: true,
      });
      applyChatProjectionForTests({
        type: "run_state_changed",
        version: 1,
        conversation_id: "conv-1",
        run_id: "run-1",
        state: runState,
      });
      const source = getChatSnapshot();
      const restore = beginConversationOpen("conv-2");
      const terminalSnapshot = {
        type: "snapshot_replaced",
        version: 1,
        conversation: conversationView(
          "conv-1",
          [{ id: "m-1", role: "user", text: "question", sequence: 1 }],
          runState === "interrupted" ? "run-1" : null,
          source.title,
        ),
      } as const;
      applyChatProjectionForTests(terminalSnapshot);
      applyChatProjectionForTests(terminalSnapshot);
      applyChatProjectionForTests({
        type: "run_state_changed",
        version: 1,
        conversation_id: "conv-1",
        run_id: "run-1",
        state: runState,
      });

      rollbackConversationOpen(restore);

      expect(getChatSnapshot()).toEqual(source);
      expect(getChatSnapshot().activeRunId).toBe("run-1");
      expect(getChatSnapshot().recoveryNotice).not.toBeNull();
      expect(getChatSnapshot().error).toBe("Provider disconnected.");
    },
  );

  it("rolls a failed open back to the authoritative snapshot that retired the source run", () => {
    bindFirstRun();
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 1, text: "partial" });
    const restore = beginConversationOpen("conv-2");
    applyInferenceForTests({ type: "completed", version: 1, run_id: "run-1", finish_reason: "stop" });
    const durable = conversationView(
      "conv-1",
      [
        { id: "m-1", role: "user", text: "question", sequence: 1 },
        { id: "m-2", role: "assistant", text: "partial", sequence: 2 },
      ],
      null,
    );
    applyChatProjectionForTests({ type: "snapshot_replaced", version: 1, conversation: durable });
    applyChatProjectionForTests({
      type: "run_state_changed",
      version: 1,
      conversation_id: "conv-1",
      run_id: "run-1",
      state: "completed",
    });

    rollbackConversationOpen(restore);

    expect(getChatSnapshot()).toMatchObject({
      conversationId: "conv-1",
      activeRunId: null,
      runState: null,
      streamingText: "",
      settlingText: "",
      activity: [],
      roundProse: [],
    });
    expect(getChatSnapshot().messages.map((message) => message.text)).toEqual(["question", "partial"]);
  });

  it("purges retained run ownership on account reset", () => {
    bindFirstRun();
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 1, text: "private" });
    openSecondConversation();

    resetChatStore();
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 2, text: "late" });
    applyToolInvocationForTests({
      type: "requested",
      version: 1,
      invocation_id: "inv-late",
      run_id: "run-1",
      tool_id: "project.read_file",
    });

    expect(getChatSnapshot()).toMatchObject({
      conversationId: null,
      activeRunId: null,
      streamingText: "",
      activity: [],
      reasoning: [],
    });
  });

  it("evicts retired reasoning before a waiting live run", () => {
    resetChatStore();
    applyConversationView(conversationView("waiting", [], "run-waiting"));
    applyChatProjectionForTests({
      type: "run_state_changed",
      version: 1,
      conversation_id: "waiting",
      run_id: "run-waiting",
      state: "awaiting_approval",
    });
    applyInferenceForTests({
      type: "text_delta",
      version: 1,
      run_id: "run-waiting",
      sequence: 1,
      text: "Before approval.",
    });
    applyToolInvocationForTests({
      type: "requested",
      version: 1,
      invocation_id: "inv-waiting",
      run_id: "run-waiting",
      tool_id: "project.read_file",
    });

    for (let index = 0; index < 13; index += 1) {
      const conversationId = `retired-${index}`;
      const runId = `run-retired-${index}`;
      applyConversationView(conversationView(conversationId, [], runId));
      applyInferenceForTests({
        type: "reasoning_summary",
        version: 1,
        run_id: runId,
        sequence: 1,
        summary: `Reasoning ${index}`,
      });
      applyInferenceForTests({ type: "completed", version: 1, run_id: runId, finish_reason: "stop" });
      applyConversationView(
        conversationView(conversationId, [
          { id: `answer-${index}`, role: "assistant", text: `Answer ${index}`, sequence: 1 },
        ]),
      );
    }

    applyToolInvocationForTests({ type: "started", version: 1, invocation_id: "inv-waiting" });
    applyToolInvocationForTests({
      type: "completed",
      version: 1,
      invocation_id: "inv-waiting",
      truncated: false,
    });
    applyRunEventForTests({ type: "state_changed", version: 1, run_id: "run-waiting", state: "running" });
    applyInferenceForTests({
      type: "text_delta",
      version: 1,
      run_id: "run-waiting",
      sequence: 2,
      text: "After approval.",
    });

    beginConversationOpen("waiting");
    expect(getChatSnapshot()).toMatchObject({
      conversationId: "waiting",
      activeRunId: "run-waiting",
      runState: "running",
      streamingText: "After approval.",
      roundProse: [{ text: "Before approval.", afterCallCount: 0 }],
    });
    expect(getChatSnapshot().activity.map((event) => event.type)).toEqual(["requested", "started", "completed"]);
    completeConversationOpen(conversationView("waiting", [], "run-waiting"), "waiting");

    const oldestRestore = beginConversationOpen("retired-0");
    expect(getChatSnapshot().reasoning).toEqual([]);
    rollbackConversationOpen(oldestRestore);
    beginConversationOpen("retired-12");
    expect(getChatSnapshot().reasoning).toEqual([{ text: "Reasoning 12", afterCallCount: 0 }]);
  });

  it("does not rescan durable history for each foreground delta", () => {
    resetChatStore();
    let sequenceReads = 0;
    const messages: ChatStoreSnapshot["messages"] = Array.from({ length: 2_000 }, (_, index) => ({
      id: `m-${index}`,
      role: "assistant",
      text: "settled",
      get sequence() {
        sequenceReads += 1;
        return BigInt(index + 1);
      },
      toolCalls: [],
    }));
    setChatStoreForTests({
      conversationId: "conv-large",
      messages,
      activeRunId: "run-large",
      runState: "running",
    });
    sequenceReads = 0;

    for (let sequence = 1; sequence <= 50; sequence += 1) {
      applyInferenceForTests({
        type: "text_delta",
        version: 1,
        run_id: "run-large",
        sequence,
        text: "x",
      });
    }

    expect(sequenceReads).toBe(0);
    expect(getChatSnapshot().streamingText).toHaveLength(50);
  });
});

describe("chatStore tool activity", () => {
  /** Puts the store in the state the app reaches after a send: conversation
   * on screen with `run-1` bound, so run-1's tool events are admitted. */
  function bindRun(conversationId = "conv-1", runId = "run-1") {
    resetChatStore();
    applyConversationView(conversationView(conversationId, [], runId));
  }

  it("records journaled invocation events in order", () => {
    bindRun();

    applyToolInvocationForTests({
      type: "requested",
      version: 1,
      invocation_id: "inv-1",
      run_id: "run-1",
      tool_id: "project.read_file",
    });
    applyToolInvocationForTests({ type: "started", version: 1, invocation_id: "inv-1" });
    applyToolInvocationForTests({
      type: "completed",
      version: 1,
      invocation_id: "inv-1",
      truncated: false,
    });

    expect(getChatSnapshot().activity.map((event) => event.type)).toEqual(["requested", "started", "completed"]);
  });

  it("drops tool events from a run that is not bound to the view", () => {
    bindRun();

    // A background conversation's run must not surface its tool calls — or
    // its approval prompt — in this view's activity feed.
    applyToolInvocationForTests({
      type: "requested",
      version: 1,
      invocation_id: "inv-foreign",
      run_id: "run-background",
      tool_id: "mcp.tenant.ida_get_memories",
    });
    applyToolInvocationForTests({
      type: "approval_required",
      version: 1,
      invocation_id: "inv-foreign",
      tool_id: "mcp.tenant.ida_get_memories",
    });

    expect(getChatSnapshot().activity).toEqual([]);
  });

  it("drops follow-up events whose invocation was never admitted", () => {
    bindRun();

    applyToolInvocationForTests({ type: "started", version: 1, invocation_id: "inv-unknown" });

    expect(getChatSnapshot().activity).toEqual([]);
  });

  it("clears activity when a new conversation begins", () => {
    bindRun();
    applyToolInvocationForTests({
      type: "requested",
      version: 1,
      invocation_id: "inv-1",
      run_id: "run-1",
      tool_id: "project.read_file",
    });

    beginNewConversation();

    expect(getChatSnapshot().activity).toEqual([]);
    // The admitted set resets with the view: a follow-up for the old
    // invocation cannot resurface in the fresh conversation.
    applyToolInvocationForTests({ type: "started", version: 1, invocation_id: "inv-1" });
    expect(getChatSnapshot().activity).toEqual([]);
  });

  it("clears activity when another conversation is opened", () => {
    // Activity is live per-run state the backend never replays into a
    // conversation view, so leaving it on screen would attribute one
    // conversation's tool calls to another.
    bindRun();
    applyToolInvocationForTests({
      type: "requested",
      version: 1,
      invocation_id: "inv-1",
      run_id: "run-1",
      tool_id: "project.read_file",
    });

    applyConversationView(conversationView("conversation-2", []));

    expect(getChatSnapshot().activity).toEqual([]);
  });

  it("preserves activity when the same conversation is re-projected mid-run", () => {
    // A mid-run reconcile (or first-message title rename) re-emits the same
    // conversation; the approval prompt lives only in `activity` and must
    // survive it.
    bindRun();
    applyToolInvocationForTests({
      type: "requested",
      version: 1,
      invocation_id: "inv-1",
      run_id: "run-1",
      tool_id: "mcp.tenant.ida_get_memories",
    });
    applyToolInvocationForTests({
      type: "approval_required",
      version: 1,
      invocation_id: "inv-1",
      tool_id: "mcp.tenant.ida_get_memories",
    });

    applyChatProjectionForTests({
      type: "snapshot_replaced",
      version: 1,
      conversation: conversationView("conv-1", [{ id: "m-1", role: "user", text: "hi", sequence: 1 }], "run-1"),
    });

    expect(getChatSnapshot().activity.map((event) => event.type)).toEqual(["requested", "approval_required"]);
  });

  it("retires the live run once its durable snapshot lands", () => {
    // The finished run's tool round comes back inside the snapshot and
    // renders as the umbrella above the answer. Keeping the live card past
    // that point drew the same round twice — once above the answer, once
    // below it (spec: chat-run-activity).
    bindRun();
    applyToolInvocationForTests({
      type: "requested",
      version: 1,
      invocation_id: "inv-1",
      run_id: "run-1",
      tool_id: "project.read_file",
    });
    applyChatProjectionForTests({
      type: "run_state_changed",
      version: 1,
      conversation_id: "conv-1",
      run_id: "run-1",
      state: "completed",
    });

    applyChatProjectionForTests({
      type: "snapshot_replaced",
      version: 1,
      conversation: conversationView(
        "conv-1",
        [
          { id: "m-1", role: "user", text: "hi", sequence: 1 },
          { id: "m-2", role: "assistant", text: "done", sequence: 2 },
        ],
        null,
      ),
    });

    expect(getChatSnapshot().activity).toEqual([]);
    expect(getChatSnapshot().roundProse).toEqual([]);
  });

  it("keeps a retired run retired when a late tool event arrives", () => {
    // The admitted-invocation set is retired with the activity it scoped, so
    // a straggling lifecycle event cannot resurrect a single-row live card
    // under the durable trail.
    bindRun();
    applyToolInvocationForTests({
      type: "requested",
      version: 1,
      invocation_id: "inv-1",
      run_id: "run-1",
      tool_id: "project.read_file",
    });
    applyChatProjectionForTests({
      type: "run_state_changed",
      version: 1,
      conversation_id: "conv-1",
      run_id: "run-1",
      state: "completed",
    });
    applyChatProjectionForTests({
      type: "snapshot_replaced",
      version: 1,
      conversation: conversationView("conv-1", [{ id: "m-1", role: "user", text: "hi", sequence: 1 }], null),
    });

    applyToolInvocationForTests({ type: "completed", version: 1, invocation_id: "inv-1", truncated: false });

    expect(getChatSnapshot().activity).toEqual([]);
  });

  it("retires the previous run's activity when the next message is sent", () => {
    // The post-submit reconcile is the first snapshot of the new exchange;
    // the settled run's card must not ride along into it.
    bindRun();
    applyToolInvocationForTests({
      type: "requested",
      version: 1,
      invocation_id: "inv-1",
      run_id: "run-1",
      tool_id: "project.read_file",
    });
    applyChatProjectionForTests({
      type: "run_state_changed",
      version: 1,
      conversation_id: "conv-1",
      run_id: "run-1",
      state: "completed",
    });

    applyChatProjectionForTests({
      type: "snapshot_replaced",
      version: 1,
      conversation: conversationView("conv-1", [{ id: "m-1", role: "user", text: "next", sequence: 1 }], null),
    });

    expect(getChatSnapshot().activity).toEqual([]);
  });

  it("bounds retained activity so a long session cannot grow without limit", () => {
    bindRun();
    for (let index = 0; index < 250; index += 1) {
      applyToolInvocationForTests({
        type: "requested",
        version: 1,
        invocation_id: `inv-${index}`,
        run_id: "run-1",
        tool_id: "project.read_file",
      });
    }

    const { activity } = getChatSnapshot();
    expect(activity.length).toBe(200);
    // The cap drops the oldest, so the most recent step is always retained.
    expect(activity[activity.length - 1].invocation_id).toBe("inv-249");
  });
});

describe("chatStore live thinking segments (spec: chat-run-activity)", () => {
  function bindRun(conversationId = "conv-1", runId = "run-1") {
    resetChatStore();
    applyConversationView(conversationView(conversationId, [], runId));
  }

  const delta = (text: string, sequence: number) =>
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence, text });

  const requested = (invocationId: string) =>
    applyToolInvocationForTests({
      type: "requested",
      version: 1,
      invocation_id: invocationId,
      run_id: "run-1",
      tool_id: "project.read_file",
    });

  it("captures the round's narration at its first tool request and empties the stream", () => {
    bindRun();
    delta("I'll check the schema. ", 1);
    requested("inv-1");

    const snapshot = getChatSnapshot();
    expect(snapshot.roundProse).toEqual([{ text: "I'll check the schema. ", afterCallCount: 0 }]);
    expect(snapshot.streamingText).toBe("");
  });

  it("parallel calls in one round capture exactly one segment", () => {
    bindRun();
    delta("Round one narration.", 1);
    requested("inv-1");
    requested("inv-2");

    const snapshot = getChatSnapshot();
    expect(snapshot.roundProse).toHaveLength(1);
    expect(snapshot.activity).toHaveLength(2);
  });

  it("a later round captures a second segment positioned after the first round's tool row", () => {
    bindRun();
    delta("Round one.", 1);
    requested("inv-1");
    applyToolInvocationForTests({ type: "completed", version: 1, invocation_id: "inv-1", truncated: false });
    delta("Round two.", 2);
    requested("inv-3");

    expect(getChatSnapshot().roundProse).toEqual([
      { text: "Round one.", afterCallCount: 0 },
      // One row, not two events: the round's `requested` and `completed` are
      // one invocation and draw one row.
      { text: "Round two.", afterCallCount: 1 },
    ]);
  });

  it("a tool request with no streamed narration captures nothing", () => {
    bindRun();
    requested("inv-1");
    expect(getChatSnapshot().roundProse).toEqual([]);
  });

  it("terminal inference keeps the captured prose — only the durable snapshot retires it", () => {
    bindRun();
    delta("Narration.", 1);
    requested("inv-1");
    applyInferenceForTests({ type: "completed", version: 1, run_id: "run-1", finish_reason: "stop" });

    // `finish_run` reconciles AFTER this event, with retries. The captured prose
    // is on screen and its durable replacement has not arrived, so clearing it
    // here blinked a round's narration off for the length of the reconcile.
    // Every live field of a run retires on the one signal (spec:
    // chat-run-activity).
    expect(getChatSnapshot().roundProse).toEqual([{ text: "Narration.", afterCallCount: 0 }]);
    expect(getChatSnapshot().streamingText).toBe("");
  });

  it("the durable snapshot is what retires it", () => {
    bindRun();
    delta("Narration.", 1);
    requested("inv-1");
    applyInferenceForTests({ type: "completed", version: 1, run_id: "run-1", finish_reason: "stop" });
    applyChatProjectionForTests({
      type: "run_state_changed",
      version: 1,
      conversation_id: "c-1" as ConversationId,
      run_id: "run-1" as RunId,
      state: "completed",
    });
    applyConversationView({
      version: 1,
      conversation_id: "c-1" as ConversationId,
      title: "t",
      availability: "live",
      messages: [
        {
          version: 1,
          message_id: "m-1",
          role: "assistant",
          text: "Narration.",
          sequence: BigInt(1),
          tool_calls: [],
          attachments: [],
        },
      ],
      active_run_id: null,
    });

    expect(getChatSnapshot().roundProse).toEqual([]);
    expect(getChatSnapshot().activity).toEqual([]);
  });
});

describe("chatStore live-answer handover (spec: chat-run-activity)", () => {
  function bindRun() {
    resetChatStore();
    applyConversationView(conversationView("conv-1", [], "run-1"));
  }

  const settledView = (text: string) =>
    conversationView(
      "conv-1",
      [
        { id: "m-1", role: "user", text: "hi", sequence: 1 },
        { id: "m-2", role: "assistant", text, sequence: 2 },
      ],
      null,
    );

  it("holds the streamed answer across the gap before its durable row lands", () => {
    // `finish_run` reconciles AFTER the terminal inference event, with
    // retries. Without the hold the answer blanks for that gap and repaints.
    bindRun();
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 1, text: "The answer." });
    applyInferenceForTests({ type: "completed", version: 1, run_id: "run-1", finish_reason: "stop" });

    const snapshot = getChatSnapshot();
    expect(snapshot.streamingText).toBe("");
    expect(snapshot.settlingText).toBe("The answer.");
  });

  it("retires the held answer the moment the durable row carries it", () => {
    // The invariant the duplicate broke: the live surfaces and the durable
    // row are never both holding the same answer.
    bindRun();
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 1, text: "The answer." });
    applyInferenceForTests({ type: "completed", version: 1, run_id: "run-1", finish_reason: "stop" });
    applyChatProjectionForTests({
      type: "run_state_changed",
      version: 1,
      conversation_id: "conv-1",
      run_id: "run-1",
      state: "completed",
    });

    applyChatProjectionForTests({ type: "snapshot_replaced", version: 1, conversation: settledView("The answer.") });

    const snapshot = getChatSnapshot();
    expect(snapshot.settlingText).toBe("");
    expect(snapshot.streamingText).toBe("");
    expect(snapshot.activity).toEqual([]);
    expect(snapshot.messages.map((message) => message.text)).toEqual(["hi", "The answer."]);
  });

  it("retires the held answer together with the activity feed, never before it", () => {
    // Both retire on ONE signal. Retiring them separately is what let the
    // live card lose the answer while the transcript grew a second copy.
    bindRun();
    applyToolInvocationForTests({
      type: "requested",
      version: 1,
      invocation_id: "inv-1",
      run_id: "run-1",
      tool_id: "project.read_file",
    });
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 1, text: "The answer." });
    applyInferenceForTests({ type: "completed", version: 1, run_id: "run-1", finish_reason: "stop" });

    const settling = getChatSnapshot();
    expect(settling.settlingText).toBe("The answer.");
    expect(settling.activity).toHaveLength(1);

    applyChatProjectionForTests({
      type: "run_state_changed",
      version: 1,
      conversation_id: "conv-1",
      run_id: "run-1",
      state: "completed",
    });
    applyChatProjectionForTests({ type: "snapshot_replaced", version: 1, conversation: settledView("The answer.") });

    const settled = getChatSnapshot();
    expect(settled.settlingText).toBe("");
    expect(settled.activity).toEqual([]);
  });

  it("drops a dead run's partial text instead of holding it as an answer", () => {
    bindRun();
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 1, text: "Half an ans" });
    applyInferenceForTests({
      type: "failed",
      version: 1,
      run_id: "run-1",
      code: "provider_unavailable",
      message: "The model provider stopped answering.",
      retryable: true,
    });

    const snapshot = getChatSnapshot();
    expect(snapshot.streamingText).toBe("");
    expect(snapshot.settlingText).toBe("");
    expect(snapshot.error).toBe("The model provider stopped answering.");
  });

  it("does not carry a held answer into another conversation", () => {
    bindRun();
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 1, text: "The answer." });
    applyInferenceForTests({ type: "completed", version: 1, run_id: "run-1", finish_reason: "stop" });

    applyConversationView(conversationView("conv-2", []));

    expect(getChatSnapshot().settlingText).toBe("");
  });
});

describe("chatStore reasoning (spec: chat-reasoning-panel)", () => {
  function bindRun() {
    resetChatStore();
    applyConversationView(conversationView("conv-1", [], "run-1"));
  }

  const reason = (sequence: number, summary: string) =>
    applyInferenceForTests({ type: "reasoning_summary", version: 1, run_id: "run-1", sequence, summary });

  const settledView = (text: string) =>
    conversationView(
      "conv-1",
      [
        { id: "m-1", role: "user", text: "hi", sequence: 1 },
        { id: "m-2", role: "assistant", text, sequence: 2 },
      ],
      null,
    );

  it("accumulates fragments in arrival order", () => {
    bindRun();
    reason(1, "First ");
    reason(2, "then.");
    expect(getChatSnapshot().reasoning).toEqual([{ text: "First then.", afterCallCount: 0 }]);
  });

  // The serial-order contract: thinking that followed a tool result belongs
  // below that result, not appended to the block that preceded it.
  it("opens a new segment once a tool row has appeared since the last fragment", () => {
    bindRun();
    reason(1, "I should look this up.");
    applyToolInvocationForTests({
      type: "requested",
      version: 1,
      invocation_id: "inv-1",
      run_id: "run-1",
      tool_id: "project.read_file",
    });
    applyToolInvocationForTests({ type: "completed", version: 1, invocation_id: "inv-1", truncated: false });
    reason(2, "Now I know.");

    expect(getChatSnapshot().reasoning).toEqual([
      { text: "I should look this up.", afterCallCount: 0 },
      { text: "Now I know.", afterCallCount: 1 },
    ]);
  });

  it("notifies subscribers once for a burst while reading exact values immediately", async () => {
    bindRun();
    let notifications = 0;
    const unsubscribe = subscribeChat(() => {
      notifications += 1;
    });

    reason(1, "a");
    reason(2, "b");
    reason(3, "c");

    // Written synchronously; only the React notification coalesces.
    expect(getChatSnapshot().reasoning).toEqual([{ text: "abc", afterCallCount: 0 }]);
    expect(notifications).toBe(0);

    await new Promise((resolve) => requestAnimationFrame(() => resolve(null)));
    expect(notifications).toBe(1);
    unsubscribe();
  });

  it("survives the durable snapshot that retires every other live surface", () => {
    bindRun();
    reason(1, "why I chose this");
    applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence: 2, text: "The answer." });
    applyInferenceForTests({ type: "completed", version: 1, run_id: "run-1", finish_reason: "stop" });
    applyChatProjectionForTests({
      type: "run_state_changed",
      version: 1,
      conversation_id: "conv-1",
      run_id: "run-1",
      state: "completed",
    });
    applyChatProjectionForTests({ type: "snapshot_replaced", version: 1, conversation: settledView("The answer.") });

    const snapshot = getChatSnapshot();
    // Nothing in the snapshot carries reasoning, so retiring it there would
    // delete the only copy that ever existed.
    expect(snapshot.settlingText).toBe("");
    expect(snapshot.activity).toEqual([]);
    expect(snapshot.reasoning).toEqual([{ text: "why I chose this", afterCallCount: 0 }]);
  });

  it("does not let the next run inherit the last one's thinking", () => {
    bindRun();
    reason(1, "old thinking");
    adoptActiveRun("conv-1", "run-2");
    expect(getChatSnapshot().reasoning).toEqual([]);
  });

  it("drops it when another conversation is opened", () => {
    bindRun();
    reason(1, "conv-1 thinking");
    applyConversationView(conversationView("conv-2", []));
    expect(getChatSnapshot().reasoning).toEqual([]);
  });

  it("drops it on a new conversation and on reset", () => {
    bindRun();
    reason(1, "thinking");
    beginNewConversation();
    expect(getChatSnapshot().reasoning).toEqual([]);

    bindRun();
    reason(1, "thinking again");
    resetChatStore();
    expect(getChatSnapshot().reasoning).toEqual([]);
    expect(getChatSnapshot().reasoningTrimmed).toBe(false);
  });
});

describe("chatStore submitted turn before its run exists", () => {
  it("claims a run for the submitted turn and releases the claim", () => {
    resetChatStore();
    beginRunSubmission();
    expect(getChatSnapshot().awaitingRun).toBe(true);

    finishRunSubmission();
    expect(getChatSnapshot().awaitingRun).toBe(false);
  });

  it("drops the claim when the reader leaves for a new conversation", () => {
    resetChatStore();
    beginRunSubmission();
    beginNewConversation();
    expect(getChatSnapshot().awaitingRun).toBe(false);
  });

  it("carries the optimistic turn into the conversation that send just created", () => {
    // The created view is empty by construction: the message it was created
    // for has not been sent yet. Applying it as an ordinary snapshot took the
    // reader's own turn off the screen for the length of the send.
    resetChatStore();
    addOptimisticUserMessage("what happened last quarter?");
    applyCreatedConversationView(conversationView("conv-new", []));

    const snapshot = getChatSnapshot();
    expect(snapshot.conversationId).toBe("conv-new");
    expect(snapshot.messages.map((message) => message.text)).toEqual(["what happened last quarter?"]);
  });

  it("lets the durable row replace the optimistic one", () => {
    resetChatStore();
    addOptimisticUserMessage("what happened last quarter?");
    applyCreatedConversationView(conversationView("conv-new", []));

    applyConversationView(
      conversationView("conv-new", [{ id: "m-1", role: "user", text: "what happened last quarter?", sequence: 1 }]),
    );

    const { messages } = getChatSnapshot();
    expect(messages).toHaveLength(1);
    expect(messages[0].id).toBe("m-1");
  });

  it("takes the turn back when the conversation could not be created", () => {
    // Nothing reached the backend, so no durable row will ever replace this
    // one — and a second attempt would otherwise carry both.
    resetChatStore();
    const optimisticId = addOptimisticUserMessage("what happened last quarter?");
    dropOptimisticUserMessage(optimisticId);
    expect(getChatSnapshot().messages).toEqual([]);
  });
});

describe("deleted conversation reconciliation", () => {
  it("rejects delayed cloud lists, views, snapshots, and creation responses", () => {
    resetChatStore();
    const view = conversationView("deleted", [{ id: "m", role: "user", text: "private", sequence: 1 }], "run-deleted");
    setWorkspaceBootstrap([summary("deleted", "Deleted"), summary("kept", "Kept")]);
    applyConversationView(view);
    applyInferenceForTests({
      type: "text_delta",
      version: 1,
      run_id: "run-deleted",
      sequence: 1,
      text: "must be purged",
    });
    removeConversationFromStore("deleted");
    const cleared = getChatSnapshot();
    expect(cleared.conversationId).toBeNull();
    expect(cleared.messages).toEqual([]);
    setWorkspaceBootstrap([summary("deleted", "Deleted"), summary("kept", "Kept")]);
    applyConversationView(view);
    applyChatProjectionForTests({ type: "snapshot_replaced", version: 1, conversation: view });
    applyCreatedConversationView(view);
    applyInferenceForTests({
      type: "text_delta",
      version: 1,
      run_id: "run-deleted",
      sequence: 2,
      text: "late",
    });
    expect(getChatSnapshot()).toEqual(cleared);
    expect(beginConversationOpen("deleted")).toBeNull();
  });

  it("does not restore a deleted background chat through an open rollback", () => {
    resetChatStore();
    setWorkspaceBootstrap([summary("first", "First"), summary("deleted", "Deleted")]);
    applyConversationView(conversationView("first", []));
    const restore = beginConversationOpen("second");
    removeConversationFromStore("deleted");
    rollbackConversationOpen(restore);
    expect(getChatSnapshot().conversationId).toBe("first");
    expect(getChatSnapshot().conversations.map((c) => c.conversation_id)).toEqual(["first"]);
  });

  it("does not restore a deleted previous chat when opening another chat fails", () => {
    resetChatStore();
    applyConversationView(conversationView("deleted", []));
    const restore = beginConversationOpen("second");
    removeConversationFromStore("deleted");
    rollbackConversationOpen(restore);
    expect(getChatSnapshot().conversationId).toBeNull();
    expect(getChatSnapshot().messages).toEqual([]);
  });
});
