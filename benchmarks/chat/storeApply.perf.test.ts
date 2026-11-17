import { existsSync, readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { reduceInferenceEvent } from "../../src/features/chat/chatReduce";
import type { ChatStoreSnapshot } from "../../src/features/chat/chatStore";

const root = dirname(fileURLToPath(import.meta.url));
const fixturePath = join(root, "fixture-10k.json");

function ensureFixture() {
  if (!existsSync(fixturePath)) {
    execFileSync("node", ["generate-fixture.mjs"], { cwd: root });
  }
  const raw = readFileSync(fixturePath, "utf8");
  return JSON.parse(raw) as {
    messages: Array<{ message_id: string; role: string; text: string; sequence: number }>;
  };
}

describe("chat store apply budget", () => {
  it("applies 10k inference deltas within the MVP budget", () => {
    ensureFixture();

    let state: ChatStoreSnapshot = {
      conversationId: "c-1",
      reservedConversationId: null,
      title: "Bench",
      availability: "live",
      messages: [],
      streamingText: "",
      settlingText: "",
      compactingRunId: null,
      activeRunId: "run-bench",
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

    const started = performance.now();
    for (let i = 0; i < 10_000; i += 1) {
      state = {
        ...state,
        ...reduceInferenceEvent(state, {
          type: "text_delta",
          version: 1,
          run_id: "run-bench",
          sequence: i,
          text: "x",
        }),
      };
    }
    const elapsed = performance.now() - started;
    expect(elapsed).toBeLessThan(500);
    expect(state.streamingText.length).toBe(10_000);
  });
});
