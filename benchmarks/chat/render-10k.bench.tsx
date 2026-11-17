import React from "react";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { act, cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MessageList } from "../../src/features/chat/MessageList";
import type { ChatMessageRow } from "../../src/features/chat/chatStore";

const root = dirname(fileURLToPath(import.meta.url));

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

function loadRows(): ChatMessageRow[] {
  const raw = readFileSync(join(root, "fixture-10k.json"), "utf8");
  const fixture = JSON.parse(raw) as {
    messages: Array<{ message_id: string; role: string; text: string; sequence: number }>;
  };
  return fixture.messages.map((message) => ({
    id: message.message_id,
    role: message.role === "user" ? "user" : "assistant",
    text: message.text,
    sequence: BigInt(message.sequence),
    toolCalls: [],
  }));
}

describe("MessageList 10k render budget", () => {
  it("does not re-estimate settled offscreen history while the answer tail changes", async () => {
    const messages = loadRows();
    const offscreen = messages[500];
    const text = offscreen.text;
    const readText = vi.fn(() => text);
    Object.defineProperty(offscreen, "text", { get: readText });
    const tree = (step: number) => (
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={messages} tail={<div>Answer frame {step}</div>} />
      </div>
    );
    const { container, rerender } = render(tree(0));
    await waitFor(() => expect(container.querySelectorAll("article").length).toBeGreaterThan(0));
    expect(container.querySelector('[data-index="500"]')).toBeNull();
    expect(readText).toHaveBeenCalled();
    readText.mockClear();

    for (let step = 1; step <= 80; step += 1) {
      act(() => rerender(tree(step)));
    }

    expect(readText).not.toHaveBeenCalled();
    expect(container).toHaveTextContent("Answer frame 80");
  });

  it("mounts 10k rows with bounded DOM and acceptable first paint time", async () => {
    const messages = loadRows();
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
    expect(container.querySelectorAll("li").length).toBeLessThan(96);
    expect(elapsed).toBeLessThan(4_000);
  });
});
