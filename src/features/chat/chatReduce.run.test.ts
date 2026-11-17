import { describe, expect, it } from "vitest";
import { reduceRunEvent } from "./chatReduce";
import type { ChatStoreSnapshot } from "./chatStore";

const base: ChatStoreSnapshot = {
  conversationId: null,
  reservedConversationId: null,
  title: "t",
  availability: null,
  messages: [],
  streamingText: "",
  settlingText: "",
  compactingRunId: null,
  activeRunId: "run-1",
  runState: "running",
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

describe("chatReduce run events", () => {
  it("surfaces recovery notice on interrupted runs", () => {
    const patch = reduceRunEvent(base, {
      type: "state_changed",
      version: 1,
      run_id: "run-1",
      state: "interrupted",
    });
    expect(patch.recoveryNotice).toContain("interrupted");
  });

  it("clears active run when completed", () => {
    const patch = reduceRunEvent(base, {
      type: "state_changed",
      version: 1,
      run_id: "run-1",
      state: "completed",
    });
    expect(patch.activeRunId).toBeNull();
    expect(patch.recoveryNotice).toBeNull();
  });
});
