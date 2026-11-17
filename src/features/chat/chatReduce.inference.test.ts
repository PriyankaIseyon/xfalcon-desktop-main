import { describe, expect, it } from "vitest";
import { reduceInferenceEvent, resetInferenceSequenceState } from "./chatReduce";
import type { ChatStoreSnapshot } from "./chatStore";

const base: ChatStoreSnapshot = {
  conversationId: "c-1",
  reservedConversationId: null,
  title: "t",
  availability: "live",
  messages: [],
  streamingText: "",
  settlingText: "",
  compactingRunId: null,
  activeRunId: "run-1",
  runState: "running",
  error: null,
  conversations: [],
  workspaceLoaded: true,
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

describe("inference sequence monotonicity", () => {
  it("drops duplicate or regressive text_delta sequences", () => {
    resetInferenceSequenceState();
    const first = reduceInferenceEvent(base, {
      type: "text_delta",
      version: 1,
      run_id: "run-1",
      sequence: 1,
      text: "A",
    });
    const state = { ...base, ...first };
    const duplicate = reduceInferenceEvent(state, {
      type: "text_delta",
      version: 1,
      run_id: "run-1",
      sequence: 1,
      text: "B",
    });
    expect(duplicate.streamingText).toBeUndefined();
    const next = reduceInferenceEvent(state, {
      type: "text_delta",
      version: 1,
      run_id: "run-1",
      sequence: 2,
      text: "C",
    });
    expect(next.streamingText).toBe("AC");
  });
});

describe("streamed reasoning", () => {
  it("appends fragments in arrival order", () => {
    resetInferenceSequenceState();
    const first = reduceInferenceEvent(base, {
      type: "reasoning_summary",
      version: 1,
      run_id: "run-1",
      sequence: 1,
      summary: "First I ",
    });
    const state = { ...base, ...first };
    const second = reduceInferenceEvent(state, {
      type: "reasoning_summary",
      version: 1,
      run_id: "run-1",
      sequence: 2,
      summary: "check the schema.",
    });
    expect(second.reasoning).toEqual([{ text: "First I check the schema.", afterCallCount: 0 }]);
    expect(second.reasoningTrimmed).toBe(false);
  });

  it("admits a replayed fragment once", () => {
    resetInferenceSequenceState();
    const first = reduceInferenceEvent(base, {
      type: "reasoning_summary",
      version: 1,
      run_id: "run-1",
      sequence: 1,
      summary: "once",
    });
    const state = { ...base, ...first };
    const replay = reduceInferenceEvent(state, {
      type: "reasoning_summary",
      version: 1,
      run_id: "run-1",
      sequence: 1,
      summary: "once",
    });
    expect(replay.reasoning).toBeUndefined();
  });

  it("ignores a fragment from another conversation's run", () => {
    resetInferenceSequenceState();
    const patch = reduceInferenceEvent(base, {
      type: "reasoning_summary",
      version: 1,
      run_id: "run-other",
      sequence: 1,
      summary: "not ours",
    });
    expect(patch).toEqual({});
  });

  // The regression the split ledger exists for: reasoning and text deltas are
  // numbered from ONE run-wide counter, so a reasoning fragment admitted
  // through the answer's ledger would advance it past the next answer delta.
  it("does not let a reasoning fragment swallow the answer delta that follows it", () => {
    resetInferenceSequenceState();
    const reasoned = reduceInferenceEvent(base, {
      type: "reasoning_summary",
      version: 1,
      run_id: "run-1",
      sequence: 7,
      summary: "thinking",
    });
    const state = { ...base, ...reasoned };
    const answer = reduceInferenceEvent(state, {
      type: "text_delta",
      version: 1,
      run_id: "run-1",
      sequence: 8,
      text: "the answer",
    });
    expect(answer.streamingText).toBe("the answer");
  });

  it("drops the oldest text and reports the trim once past the cap", () => {
    resetInferenceSequenceState();
    const seeded = { ...base, reasoning: [{ text: "A".repeat(32 * 1024), afterCallCount: 0 }] };
    const patch = reduceInferenceEvent(seeded, {
      type: "reasoning_summary",
      version: 1,
      run_id: "run-1",
      sequence: 1,
      summary: "NEWEST",
    });
    expect(patch.reasoningTrimmed).toBe(true);
    const kept = patch.reasoning ?? [];
    expect(kept.reduce((total, segment) => total + segment.text.length, 0)).toBe(32 * 1024);
    expect(kept[kept.length - 1].text.endsWith("NEWEST")).toBe(true);
  });

  it("rejects late reasoning after the run reached a terminal event", () => {
    resetInferenceSequenceState();
    const state = {
      ...base,
      ...reduceInferenceEvent(base, {
        type: "reasoning_summary",
        version: 1,
        run_id: "run-1",
        sequence: 9,
        summary: "old",
      }),
    };
    reduceInferenceEvent(state, { type: "completed", version: 1, run_id: "run-1", finish_reason: "stop" });
    const afterTerminal = reduceInferenceEvent(state, {
      type: "reasoning_summary",
      version: 1,
      run_id: "run-1",
      sequence: 1,
      summary: "new",
    });
    expect(afterTerminal).toEqual({});
  });
});

describe("terminal inference events", () => {
  it("clears the stream and keeps no error on completion", () => {
    resetInferenceSequenceState();
    const patch = reduceInferenceEvent(base, {
      type: "completed",
      version: 1,
      run_id: "run-1",
      finish_reason: "stop",
    });
    expect(patch.streamingText).toBe("");
    expect(patch.error).toBeUndefined();
  });

  it("surfaces the failure message instead of dropping it", () => {
    resetInferenceSequenceState();
    const patch = reduceInferenceEvent(base, {
      type: "failed",
      version: 1,
      run_id: "run-1",
      code: "provider_unavailable",
      message: "The agent run did not finish cleanly. You can retry.",
      retryable: true,
    });
    expect(patch.streamingText).toBe("");
    expect(patch.error).toBe("The agent run did not finish cleanly. You can retry.");
  });
});

describe("what a terminal inference event retires (spec: chat-run-activity)", () => {
  const withRound: ChatStoreSnapshot = {
    ...base,
    streamingText: "Net sales came to $75.6M.",
    roundProse: [{ text: "I'll check the sales table.", afterCallCount: 0 }],
  };

  it("holds the answer AND the round prose across the settle gap", () => {
    resetInferenceSequenceState();
    const patch = reduceInferenceEvent(withRound, {
      type: "completed",
      version: 1,
      run_id: "run-1",
      finish_reason: "stop",
    });

    // `finish_run` reconciles after this event, with retries. Both pieces of
    // on-screen prose have to survive that gap, or they blink off and back on.
    expect(patch.streamingText).toBe("");
    expect(patch.settlingText).toBe("Net sales came to $75.6M.");
    expect(patch.roundProse).toBeUndefined();
  });

  it("keeps a dead run's completed rounds while dropping its partial answer", () => {
    resetInferenceSequenceState();
    const patch = reduceInferenceEvent(withRound, {
      type: "failed",
      version: 1,
      run_id: "run-1",
      code: "internal",
      message: "It failed.",
      retryable: true,
    });

    // The partial answer is not an answer. The rounds that finished did happen,
    // and `activity` is not cleared here either — the notice should not explain a
    // failure with no visible history behind it.
    expect(patch.streamingText).toBe("");
    expect(patch.settlingText).toBe("");
    expect(patch.roundProse).toBeUndefined();
    expect(patch.error).toBe("It failed.");
  });
});

describe("context compaction notice", () => {
  it("marks the run while it compacts and clears when it ends, completes or fails", () => {
    resetInferenceSequenceState();
    const event = (active: boolean) => ({ type: "context_compaction", version: 1, run_id: "run-1", active }) as const;
    const started = { ...base, ...reduceInferenceEvent(base, event(true)) };
    expect(started.compactingRunId).toBe("run-1");
    expect({ ...started, ...reduceInferenceEvent(started, event(false)) }.compactingRunId).toBeNull();
    expect(
      reduceInferenceEvent(started, { type: "completed", version: 1, run_id: "run-1", finish_reason: "stop" })
        .compactingRunId,
    ).toBeNull();
    resetInferenceSequenceState();
    expect(
      reduceInferenceEvent(started, {
        type: "failed",
        version: 1,
        run_id: "run-1",
        code: "internal",
        message: "x",
        retryable: false,
      }).compactingRunId,
    ).toBeNull();
  });

  it("drops a notice from a run that is not the active one", () => {
    const patch = reduceInferenceEvent(base, { type: "context_compaction", version: 1, run_id: "run-9", active: true });
    expect(patch).toEqual({});
  });
});
