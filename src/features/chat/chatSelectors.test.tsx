import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { ChatConversationView, ConversationId, RunId } from "../../lib/generated/contracts";
import { applyConversationView, applyInferenceForTests, getChatSnapshot, resetChatStore } from "./chatStore";
import { setComposerDraft } from "./composerStore";
import { useChat } from "./useChat";
import { useTranscript } from "./useTranscript";

afterEach(() => {
  cleanup();
  resetChatStore();
});

function conversationView(runId: string | null): ChatConversationView {
  return {
    version: 1,
    conversation_id: "conv-1" as ConversationId,
    title: "Selector fixture",
    availability: "live",
    messages: [
      {
        version: 1,
        message_id: "m-1",
        role: "user",
        text: "hello",
        sequence: BigInt(1),
        tool_calls: [],
        attachments: [],
      },
    ],
    active_run_id: (runId as RunId | null) ?? null,
  };
}

function streamDelta(sequence: number, text: string) {
  applyInferenceForTests({ type: "text_delta", version: 1, run_id: "run-1", sequence, text });
}

/** The store commits a streamed delta synchronously and notifies on the next
 * animation frame, so a test that asserts on renders has to reach that frame. */
async function flushFrame() {
  await act(async () => {
    await new Promise((resolve) => requestAnimationFrame(() => resolve(null)));
  });
}

describe("chat store selectors (spec: chat-streaming-performance)", () => {
  it("keeps a selected reference stable across an unrelated change", async () => {
    resetChatStore();
    applyConversationView(conversationView("run-1"));
    const before = getChatSnapshot().messages;

    streamDelta(1, "answering");
    await flushFrame();

    // `useSyncExternalStore` compares by reference, so a selector whose value
    // changed identity for an unrelated reason would re-render every subscriber
    // reading it — the exact cost this split exists to remove.
    expect(getChatSnapshot().messages).toBe(before);
  });

  it("does not re-render the shell for a streamed delta", async () => {
    resetChatStore();
    applyConversationView(conversationView("run-1"));

    let renders = 0;
    renderHook(() => {
      renders += 1;
      return useChat(false);
    });
    const settled = renders;

    streamDelta(1, "one ");
    await flushFrame();
    streamDelta(2, "two ");
    await flushFrame();

    expect(renders).toBe(settled);
  });

  it("re-renders the transcript for a streamed delta", async () => {
    resetChatStore();
    applyConversationView(conversationView("run-1"));

    const { result } = renderHook(() => useTranscript());
    expect(result.current.liveText).toBe("");

    streamDelta(1, "the answer");
    await flushFrame();

    expect(result.current.liveText).toBe("the answer");
  });

  it("does not re-render the shell for a keystroke", () => {
    resetChatStore();

    let renders = 0;
    renderHook(() => {
      renders += 1;
      return useChat(false);
    });
    const settled = renders;

    act(() => {
      setComposerDraft("typing");
      setComposerDraft("typing more");
    });

    expect(renders).toBe(settled);
  });
});
