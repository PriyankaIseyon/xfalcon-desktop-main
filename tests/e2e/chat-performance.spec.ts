import { expect, test } from "@playwright/test";
import type { InferenceEvent } from "../../src/lib/generated/contracts";
import { installChatMocks } from "./chat-mocks";

test("long history stays bounded and the composer stays usable during a large answer", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1100, height: 800 });
  await installChatMocks(page, {
    bootstrapConversations: [{ conversation_id: "long-chat", title: "Long conversation" }],
    conversationActiveRunId: "long-run",
    conversationMessages: Array.from({ length: 10_000 }, (_, index) => ({
      message_id: `history-${index}`,
      role: index % 2 === 0 ? ("user" as const) : ("assistant" as const),
      text: `History ${index}: revenue, margin, inventory and stock coverage remain available.`,
      sequence: index + 1,
    })),
  });
  await page.goto("/");
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 4 });
  await page.getByRole("option", { name: "Long conversation" }).click();
  await expect(page.getByRole("button", { name: "Stop the assistant's response" })).toBeVisible();
  const answer = page.locator('[aria-live="polite"] .assistant-markdown');
  const source = Array.from(
    { length: 120 },
    (_, index) =>
      `Paragraph ${index}. ${"Revenue, margin and inventory stay readable while new text arrives. ".repeat(4)}`,
  ).join("\n\n");
  const profilePromise = page.evaluate(async (text) => {
    const mock = (
      window as Window & {
        __E2E_CHAT_MOCK__?: { emitInferenceEvent: (event: InferenceEvent) => void };
      }
    ).__E2E_CHAT_MOCK__;
    if (!mock) throw new Error("missing chat mock");
    const frameGaps: number[] = [];
    const longTasks: number[] = [];
    const observer = new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) longTasks.push(entry.duration);
    });
    observer.observe({ type: "longtask" });
    let previousFrame = performance.now();
    let sequence = 0;
    for (let offset = 0; offset < text.length; offset += 256) {
      await new Promise<void>((resolve) =>
        requestAnimationFrame((now) => {
          frameGaps.push(now - previousFrame);
          previousFrame = now;
          resolve();
        }),
      );
      mock.emitInferenceEvent({
        type: "text_delta",
        version: 1,
        run_id: "long-run",
        sequence: ++sequence,
        text: text.slice(offset, offset + 256),
      });
    }
    await new Promise<void>((resolve) => setTimeout(resolve, 100));
    observer.disconnect();
    frameGaps.sort((a, b) => a - b);
    return {
      cpuThrottleRate: 4,
      frames: frameGaps.length,
      p50FrameMs: frameGaps[Math.floor(frameGaps.length * 0.5)],
      p95FrameMs: frameGaps[Math.floor(frameGaps.length * 0.95)],
      maxFrameMs: frameGaps.at(-1),
      longTasks,
      mountedHistoryRows: document.querySelectorAll(".chat-column-messages [data-index]").length,
    };
  }, source);

  const composer = page.getByRole("textbox", { name: "Message xFalcon" });
  await composer.fill("Keep this draft while the answer streams.");
  await expect(composer).toHaveValue("Keep this draft while the answer streams.");
  const profile = await profilePromise;
  await expect(answer.locator(":scope > p")).toHaveText(source.split("\n\n"));
  expect(profile.mountedHistoryRows).toBeLessThan(96);
  await expect(composer).toHaveValue("Keep this draft while the answer streams.");

  await page.evaluate(() => {
    const mock = (window as Window & { __E2E_CHAT_MOCK__?: { emitInferenceEvent: (event: InferenceEvent) => void } })
      .__E2E_CHAT_MOCK__;
    if (!mock) throw new Error("missing chat mock");
    mock.emitInferenceEvent({ type: "completed", version: 1, run_id: "long-run", finish_reason: "stop" });
  });
  await expect(page.locator(".streaming-text-caret")).toHaveCount(0);
  await expect(composer).toHaveValue("Keep this draft while the answer streams.");
  console.info(`[chat-long] ${JSON.stringify(profile)}`);
  await testInfo.attach("long-chat-frame-profile", {
    body: JSON.stringify(profile, null, 2),
    contentType: "application/json",
  });
});
