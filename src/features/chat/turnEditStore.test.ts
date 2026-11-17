import { describe, expect, it } from "vitest";
import { clearTurnEdit, getTurnEditSnapshot, startTurnEdit, updateTurnEditDraft } from "./turnEditStore";

describe("turnEditStore", () => {
  it("starts in an inactive empty state", () => {
    clearTurnEdit();
    const snapshot = getTurnEditSnapshot();
    expect(snapshot.activeMessageId).toBeNull();
    expect(snapshot.draft).toBe("");
  });

  it("activates edit mode with message id and draft text", () => {
    startTurnEdit("msg-42", "How many customers?");
    const snapshot = getTurnEditSnapshot();
    expect(snapshot.activeMessageId).toBe("msg-42");
    expect(snapshot.draft).toBe("How many customers?");
  });

  it("updates draft text in place", () => {
    startTurnEdit("msg-42", "How many customers?");
    updateTurnEditDraft("How many customers in Q3?");
    const snapshot = getTurnEditSnapshot();
    expect(snapshot.activeMessageId).toBe("msg-42");
    expect(snapshot.draft).toBe("How many customers in Q3?");
  });

  it("clears edit mode completely", () => {
    startTurnEdit("msg-42", "How many customers?");
    clearTurnEdit();
    const snapshot = getTurnEditSnapshot();
    expect(snapshot.activeMessageId).toBeNull();
    expect(snapshot.draft).toBe("");
  });
});

import {
  applyConversationView,
  applyInferenceForTests,
  beginTurnRevision,
  rollbackTurnRevision,
  finishTurnRevision,
  getChatSnapshot,
  resetChatStore,
  setChatStoreForTests,
} from "./chatStore";

describe("chatStore turn revision and rollback", () => {
  it("clears a draft when a different conversation replaces the transcript", () => {
    setChatStoreForTests({ conversationId: "c-1" });
    startTurnEdit("m-1", "Draft for conversation one");

    applyConversationView({
      version: 1,
      conversation_id: "c-2",
      title: "Conversation two",
      availability: "live",
      messages: [],
      active_run_id: null,
    });

    expect(getTurnEditSnapshot()).toEqual({ activeMessageId: null, draft: "" });
  });

  it("cuts messages optimistically and restores full snapshot including reasoning and activity on rollback", () => {
    setChatStoreForTests({
      conversationId: "c-1",
      title: "Title",
      availability: "live",
      messages: [
        { id: "m-1", role: "user", text: "Q1", sequence: 1n, toolCalls: [] },
        { id: "m-2", role: "assistant", text: "A1", sequence: 2n, toolCalls: [] },
        { id: "m-3", role: "user", text: "Q2", sequence: 3n, toolCalls: [] },
        { id: "m-4", role: "assistant", text: "A2", sequence: 4n, toolCalls: [] },
      ],
      streamingText: "",
      settlingText: "",
      activeRunId: "run-4",
      runState: "completed",
      error: null,
      conversations: [],
      workspaceLoaded: true,
      recoveryNotice: null,
      activity: [
        {
          type: "requested",
          version: 1,
          invocation_id: "inv-1",
          run_id: "run-4",
          tool_id: "test_tool",
        },
      ],
      roundProse: [{ afterCallCount: 0, text: "Round 1 prose" }],
      reasoning: [{ afterCallCount: 0, text: "Thinking step 1" }],
      reasoningTrimmed: true,
      revising: false,
    });
    const initialState = getChatSnapshot();

    // Begin revision at m-3 (user question 2)
    const restore = beginTurnRevision("m-3");
    expect(restore).not.toBeNull();

    const duringRevision = getChatSnapshot();
    expect(duringRevision.revising).toBe(true);
    expect(duringRevision.messages).toHaveLength(2);
    expect(duringRevision.messages.map((m) => m.id)).toEqual(["m-1", "m-2"]);
    expect(duringRevision.activity).toHaveLength(0);
    expect(duringRevision.reasoning).toHaveLength(0);

    // Rollback revision
    rollbackTurnRevision(restore!);

    const afterRollback = getChatSnapshot();
    expect(afterRollback).toEqual(initialState);
    expect(afterRollback.revising).toBe(false);
    expect(afterRollback.messages).toHaveLength(4);
    expect(afterRollback.messages.map((m) => m.id)).toEqual(["m-1", "m-2", "m-3", "m-4"]);
    expect(afterRollback.activity).toHaveLength(1);
    expect(afterRollback.reasoning).toHaveLength(1);
    expect(afterRollback.reasoningTrimmed).toBe(true);
    expect(afterRollback.activeRunId).toBe("run-4");
  });

  it("restores the text sequence watermark when a revision rolls back", () => {
    resetChatStore();
    applyConversationView({
      version: 1,
      conversation_id: "c-text",
      title: "Text rollback",
      availability: "live",
      messages: [
        {
          version: 1,
          message_id: "m-1",
          role: "user",
          text: "Question",
          sequence: 1n,
          tool_calls: [],
          attachments: [],
        },
      ],
      active_run_id: "run-text",
    });
    const delta = {
      type: "text_delta",
      version: 1,
      run_id: "run-text",
      sequence: 7,
      text: "Answer once.",
    } as const;
    applyInferenceForTests(delta);
    const before = getChatSnapshot();

    const restore = beginTurnRevision("m-1");
    rollbackTurnRevision(restore);
    applyInferenceForTests(delta);

    expect(getChatSnapshot()).toEqual(before);
  });

  it("restores the reasoning sequence watermark when a revision rolls back", () => {
    resetChatStore();
    applyConversationView({
      version: 1,
      conversation_id: "c-reasoning",
      title: "Reasoning rollback",
      availability: "live",
      messages: [
        {
          version: 1,
          message_id: "m-1",
          role: "user",
          text: "Question",
          sequence: 1n,
          tool_calls: [],
          attachments: [],
        },
      ],
      active_run_id: "run-reasoning",
    });
    const delta = {
      type: "reasoning_summary",
      version: 1,
      run_id: "run-reasoning",
      sequence: 9,
      summary: "Think once.",
    } as const;
    applyInferenceForTests(delta);
    const before = getChatSnapshot();

    const restore = beginTurnRevision("m-1");
    rollbackTurnRevision(restore);
    applyInferenceForTests(delta);

    expect(getChatSnapshot()).toEqual(before);
  });

  it("finishes revision cleanly", () => {
    setChatStoreForTests({
      conversationId: "c-1",
      messages: [{ id: "m-1", role: "user", text: "Q1", sequence: 1n, toolCalls: [] }],
      revising: true,
    });

    finishTurnRevision("c-1");
    expect(getChatSnapshot().revising).toBe(false);
  });

  it("does not let the revised run's late stream resurrect retired live state", () => {
    setChatStoreForTests({
      conversationId: "c-revised",
      messages: [
        { id: "m-1", role: "user", text: "Q1", sequence: 1n, toolCalls: [] },
        { id: "m-2", role: "assistant", text: "A1", sequence: 2n, toolCalls: [] },
      ],
      activeRunId: "run-revised",
      runState: "running",
      streamingText: "old partial",
      revising: false,
      awaitingRun: false,
    });

    const restore = beginTurnRevision("m-2");
    expect(restore).not.toBeNull();
    finishTurnRevision("c-revised");
    applyInferenceForTests({
      type: "text_delta",
      version: 1,
      run_id: "run-revised",
      sequence: 1,
      text: "late",
    });

    expect(getChatSnapshot()).toMatchObject({
      conversationId: "c-revised",
      activeRunId: null,
      streamingText: "",
      revising: false,
    });
  });

  it("does not clear a stale revision's flag once the reader has moved to another conversation", () => {
    setChatStoreForTests({
      conversationId: "c-1",
      messages: [{ id: "m-1", role: "user", text: "Q1", sequence: 1n, toolCalls: [] }],
      revising: true,
    });

    // The reader switches away before c-1's edit resolves; the new
    // conversation starts its own revision.
    applyConversationView({
      version: 1,
      conversation_id: "c-2",
      title: "Conversation two",
      availability: "live",
      messages: [],
      active_run_id: null,
    });
    setChatStoreForTests({ revising: true });

    // c-1's edit finally succeeds and reports it is finished.
    finishTurnRevision("c-1");

    expect(getChatSnapshot().conversationId).toBe("c-2");
    expect(getChatSnapshot().revising).toBe(true);
  });

  it("refuses a second optimistic cut while the first restore point is active", () => {
    setChatStoreForTests({
      messages: [
        { id: "m-1", role: "user", text: "Q1", sequence: 1n, toolCalls: [] },
        { id: "m-2", role: "assistant", text: "A1", sequence: 2n, toolCalls: [] },
        { id: "m-3", role: "user", text: "Q2", sequence: 3n, toolCalls: [] },
        { id: "m-4", role: "assistant", text: "A2", sequence: 4n, toolCalls: [] },
      ],
      revising: false,
      awaitingRun: false,
    });

    const restore = beginTurnRevision("m-3");
    expect(restore).not.toBeNull();
    expect(beginTurnRevision("m-1")).toBeNull();

    rollbackTurnRevision(restore);
    expect(getChatSnapshot().messages.map((message) => message.id)).toEqual(["m-1", "m-2", "m-3", "m-4"]);
  });

  it("does not restore a stale revision's snapshot over a conversation switched to in the meantime", () => {
    setChatStoreForTests({
      conversationId: "c-1",
      messages: [
        { id: "m-1", role: "user", text: "Q1", sequence: 1n, toolCalls: [] },
        { id: "m-2", role: "assistant", text: "A1", sequence: 2n, toolCalls: [] },
      ],
      conversations: [],
      workspaceLoaded: true,
      revising: false,
      awaitingRun: false,
    });

    // Edit is submitted on c-1; its network round trip is still pending.
    const restore = beginTurnRevision("m-2");
    expect(restore).not.toBeNull();

    // The user navigates away before the edit's backend answer lands.
    applyConversationView({
      version: 1,
      conversation_id: "c-2",
      title: "Conversation two",
      availability: "live",
      messages: [
        {
          version: 1,
          message_id: "m-9",
          role: "user",
          text: "c-2's own message",
          sequence: 1n,
          tool_calls: [],
          attachments: [],
        },
      ],
      active_run_id: null,
    });
    const onC2 = getChatSnapshot();
    expect(onC2.conversationId).toBe("c-2");

    // c-1's edit now reports failure and rolls back -- against a chat that
    // has moved on to a different conversation.
    rollbackTurnRevision(restore);

    const afterRollback = getChatSnapshot();
    expect(afterRollback.conversationId).toBe("c-2");
    expect(afterRollback.messages).toEqual(onC2.messages);
  });
});
