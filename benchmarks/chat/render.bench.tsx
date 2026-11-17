import React from "react";
import { cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MessageList } from "../../src/features/chat/MessageList";
import type { ChatMessageRow } from "../../src/features/chat/chatStore";

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
});

function row(index: number): ChatMessageRow {
  return {
    id: `bench-${index}`,
    role: index % 2 === 0 ? "user" : "assistant",
    text: `Bench message ${index} with mixed content`,
    sequence: BigInt(index),
    toolCalls: [],
  };
}

describe("MessageList render budget", () => {
  it("mounts a 500-row transcript with bounded DOM rows", async () => {
    const messages = Array.from({ length: 500 }, (_, index) => row(index));
    const started = performance.now();
    const { container } = render(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={messages} />
      </div>,
    );
    await waitFor(() => {
      expect(container.querySelectorAll("article").length).toBeGreaterThan(0);
    });
    const elapsed = performance.now() - started;
    const mounted = container.querySelectorAll("li").length;
    expect(mounted).toBeLessThan(64);
    expect(elapsed).toBeLessThan(2_000);
  });
});
