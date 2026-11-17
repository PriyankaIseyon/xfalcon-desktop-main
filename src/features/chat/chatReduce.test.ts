import { beforeEach, describe, expect, it } from "vitest";
import { reduceInferenceEvent, reduceRunEvent, resetInferenceSequenceState } from "./chatReduce";
import type { ChatStoreSnapshot } from "./chatStore";

const base: ChatStoreSnapshot = {
  conversationId: "conv-1",
  reservedConversationId: null,
  title: "t",
  availability: null,
  messages: [],
  streamingText: "",
  settlingText: "",
  compactingRunId: null,
  // Events are admitted only for the run bound to the view; the tests bind
  // run-1 the way the app does (submit result / run_state_changed projection).
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

describe("chatReduce", () => {
  beforeEach(() => resetInferenceSequenceState());

  it("appends text deltas and clears on completion", () => {
    const afterDelta = {
      ...base,
      ...reduceInferenceEvent(base, {
        type: "text_delta",
        version: 1,
        run_id: "run-1",
        sequence: 1,
        text: "Hi",
      }),
    };
    expect(afterDelta.streamingText).toBe("Hi");
    const afterMore = {
      ...afterDelta,
      ...reduceInferenceEvent(afterDelta, {
        type: "text_delta",
        version: 1,
        run_id: "run-1",
        sequence: 2,
        text: " there",
      }),
    };
    expect(afterMore.streamingText).toBe("Hi there");
    const cleared = {
      ...afterMore,
      ...reduceInferenceEvent(afterMore, {
        type: "completed",
        version: 1,
        run_id: "run-1",
        finish_reason: "stop",
      }),
    };
    expect(cleared.streamingText).toBe("");
  });

  it("ignores duplicate or out-of-order text_delta sequence per run", () => {
    const afterFirst = {
      ...base,
      ...reduceInferenceEvent(base, {
        type: "text_delta",
        version: 1,
        run_id: "run-1",
        sequence: 2,
        text: "ab",
      }),
    };
    expect(afterFirst.streamingText).toBe("ab");
    const replay = {
      ...afterFirst,
      ...reduceInferenceEvent(afterFirst, {
        type: "text_delta",
        version: 1,
        run_id: "run-1",
        sequence: 2,
        text: "replay",
      }),
    };
    expect(replay.streamingText).toBe("ab");
    const afterThird = {
      ...replay,
      ...reduceInferenceEvent(replay, {
        type: "text_delta",
        version: 1,
        run_id: "run-1",
        sequence: 3,
        text: "c",
      }),
    };
    expect(afterThird.streamingText).toBe("abc");
  });

  it("drops inference events from a run that is not bound to the view", () => {
    const foreignDelta = reduceInferenceEvent(base, {
      type: "text_delta",
      version: 1,
      run_id: "run-background",
      sequence: 1,
      text: "leaked from another conversation",
    });
    expect(foreignDelta).toEqual({});

    const foreignFailure = reduceInferenceEvent(base, {
      type: "failed",
      version: 1,
      run_id: "run-background",
      code: "provider_unavailable",
      message: "boom",
      retryable: false,
    });
    expect(foreignFailure).toEqual({});

    const unbound = reduceInferenceEvent(
      { ...base, activeRunId: null },
      {
        type: "text_delta",
        version: 1,
        run_id: "run-1",
        sequence: 1,
        text: "no run adopted yet",
      },
    );
    expect(unbound).toEqual({});
  });

  it("tracks state changes for the bound run only", () => {
    const next = {
      ...base,
      ...reduceRunEvent(base, {
        type: "state_changed",
        version: 1,
        run_id: "run-1",
        state: "running",
      }),
    };
    expect(next.activeRunId).toBe("run-1");
    expect(next.runState).toBe("running");

    // A background run's lifecycle must not flip this view's pending state —
    // it once disabled the composer and starter prompts on a brand-new chat.
    const foreign = reduceRunEvent(next, {
      type: "state_changed",
      version: 1,
      run_id: "run-background",
      state: "running",
    });
    expect(foreign).toEqual({});

    const unbound = reduceRunEvent(
      { ...base, activeRunId: null },
      {
        type: "state_changed",
        version: 1,
        run_id: "run-1",
        state: "running",
      },
    );
    expect(unbound).toEqual({});
  });
});
