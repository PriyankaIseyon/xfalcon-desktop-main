import React from "react";
import { existsSync, readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { act, cleanup, render, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MessageList } from "../../src/features/chat/MessageList";
import { getChatSnapshot, resetChatStore } from "../../src/features/chat/chatStore";
import { useChat } from "../../src/features/chat/useChat";
import type { ChatConversationView, ConversationId } from "../../src/lib/generated/contracts";
import { chatConversationOpen, chatConversationPreview } from "../../src/lib/native";

const root = dirname(fileURLToPath(import.meta.url));
const fixturePath = join(root, "fixture-10k.json");
const benchConversationId = "conv-10k-open-bench" as ConversationId;

vi.mock("../../src/lib/native", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../src/lib/native")>();
  return {
    ...actual,
    chatConversationOpen: vi.fn(),
    chatConversationPreview: vi.fn(),
  };
});

afterEach(cleanup);

beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    width: 640,
    height: 480,
    top: 0,
    left: 0,
    bottom: 480,
    right: 640,
    x: 0,
    y: 0,
    toJSON: () => ({}),
  });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get: () => 480,
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  resetChatStore();
  vi.mocked(chatConversationOpen).mockReset();
  vi.mocked(chatConversationPreview).mockReset();
});

function ensureFixture() {
  if (!existsSync(fixturePath)) {
    execFileSync("node", ["generate-fixture.mjs"], { cwd: root });
  }
  const raw = readFileSync(fixturePath, "utf8");
  return JSON.parse(raw) as {
    messages: Array<{ message_id: string; role: string; text: string; sequence: number }>;
  };
}

function conversationViewFromFixture(): ChatConversationView {
  const fixture = ensureFixture();
  return {
    version: 1,
    conversation_id: benchConversationId,
    title: "Open 10k bench",
    availability: "live",
    messages: fixture.messages.map((message) => ({
      version: 1,
      message_id: message.message_id,
      role: message.role === "user" ? "user" : "assistant",
      text: message.text,
      sequence: BigInt(message.sequence),
      tool_calls: [],
      attachments: [],
    })),
    active_run_id: null,
  };
}

describe("chat_conversation_open 10k gate", () => {
  it("applies a 10k mock open view and mounts bounded MessageList rows", async () => {
    const view = conversationViewFromFixture();
    expect(view.messages).toHaveLength(10_000);
    vi.mocked(chatConversationPreview).mockResolvedValue(null);
    vi.mocked(chatConversationOpen).mockResolvedValue(view);

    resetChatStore();
    const { result } = renderHook(() => useChat(true));

    const started = performance.now();
    await act(async () => {
      await result.current.openConversation(benchConversationId);
    });
    const openElapsed = performance.now() - started;

    expect(chatConversationOpen).toHaveBeenCalledWith(benchConversationId);
    const snapshot = getChatSnapshot();
    expect(snapshot.conversationId).toBe(benchConversationId);
    expect(snapshot.messages).toHaveLength(10_000);
    expect(openElapsed).toBeLessThan(2_000);

    const { container } = render(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={result.current.messages} />
      </div>,
    );
    await waitFor(() => {
      expect(container.querySelectorAll("article").length).toBeGreaterThan(0);
    });
    expect(container.querySelectorAll("li").length).toBeLessThan(96);
  });
});
