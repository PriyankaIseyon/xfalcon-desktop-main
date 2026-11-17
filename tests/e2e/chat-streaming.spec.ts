import { expect, test } from "@playwright/test";
import type { ChatProjectionEvent, InferenceEvent } from "../../src/lib/generated/contracts";
import { getChatMockInvocations, installChatMocks } from "./chat-mocks";

for (const reducedMotion of ["no-preference", "reduce"] as const) {
  test(`live answers continue across conversation switches (${reducedMotion})`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion });
    await installChatMocks(page, {
      bootstrapConversations: [
        { conversation_id: "live", title: "Live conversation" },
        { conversation_id: "other", title: "Other conversation" },
      ],
      conversationActiveRunIds: { live: "run-live", other: null },
    });
    await page.goto("/");
    await page.getByRole("option", { name: "Live conversation" }).click();
    await expect(page.getByRole("button", { name: "Stop the assistant's response" })).toBeVisible();

    const emit = async (events: InferenceEvent[]) => {
      await page.evaluate((payloads) => {
        const mock = (
          window as Window & { __E2E_CHAT_MOCK__?: { emitInferenceEvent: (event: InferenceEvent) => void } }
        ).__E2E_CHAT_MOCK__;
        if (!mock) throw new Error("missing chat mock");
        for (const event of payloads) mock.emitInferenceEvent(event);
      }, events);
    };
    const prefix = "First answer starts.";
    await emit([{ type: "text_delta", version: 1, run_id: "run-live", sequence: 1, text: prefix }]);
    const answer = page.locator(".assistant-markdown");
    await expect(answer).toHaveText(prefix);

    await page.getByRole("option", { name: "Other conversation" }).click();
    await expect(answer).toHaveCount(0);
    await expect(page.getByRole("textbox", { name: "Message xFalcon" })).toBeEnabled();
    await expect(page.getByRole("button", { name: "Stop the assistant's response" })).toHaveCount(0);
    const continuation = " Continues while I read another chat.";
    const delta: InferenceEvent = {
      type: "text_delta",
      version: 1,
      run_id: "run-live",
      sequence: 2,
      text: continuation,
    };
    await emit([delta, delta]);
    await expect(answer).toHaveCount(0);

    await page.getByRole("option", { name: "Live conversation" }).click();
    await expect(answer).toHaveText(prefix + continuation);
    await expect(page.getByRole("button", { name: "Stop the assistant's response" })).toBeVisible();
    const ending = " Completes smoothly 🚀.";
    await emit([
      { type: "text_delta", version: 1, run_id: "run-live", sequence: 3, text: ending },
      { type: "completed", version: 1, run_id: "run-live", finish_reason: "stop" },
    ]);
    await expect(answer).toHaveText(prefix + continuation + ending);
    await expect(answer).toHaveCount(1);
    await expect(page.locator(".streaming-text-caret")).toHaveCount(0);
  });

  test(`bursty answers stay complete through streaming, settling, and reopen (${reducedMotion})`, async ({ page }) => {
    await page.setViewportSize({ width: 1100, height: 800 });
    await page.emulateMedia({ reducedMotion });
    await installChatMocks(page, { emitRunningOnSend: true });
    await page.goto("/");
    const composer = page.getByRole("textbox", { name: "Message xFalcon" });
    await composer.fill("Show the full analysis");
    await page.getByRole("button", { name: "Send message" }).click();
    await expect(page.getByRole("button", { name: "Stop the assistant's response" })).toBeVisible();

    const paragraphs = Array.from(
      { length: 120 },
      (_, index) => `Paragraph ${index}: ${"Revenue, inventory and margin remain visible. ".repeat(5)}Unicode 🚀 東京.`,
    );
    let source = [
      "## Complete analysis",
      ...paragraphs,
      "```\nfirst  line\n  indented line\n```",
      "```sql\nSELECT 'complete' AS result;\n```",
      "| Metric | Value |\n| --- | --- |\n| Revenue | 100 |",
      "End of complete answer.",
    ].join("\n\n");

    await page.evaluate(async (text) => {
      const mock = (window as Window & { __E2E_CHAT_MOCK__?: { emitInferenceEvent: (event: InferenceEvent) => void } })
        .__E2E_CHAT_MOCK__;
      if (!mock) throw new Error("missing chat mock");
      let sequence = 0;
      for (let offset = 0; offset < text.length; offset += 2048) {
        mock.emitInferenceEvent({
          type: "text_delta",
          version: 1,
          run_id: "run-e2e-1",
          sequence: ++sequence,
          text: text.slice(offset, offset + 2048),
        });
        await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
      }
    }, source);

    // The source must catch up while the run is still active, not only when
    // its terminal event skips the reveal queue.
    const answer = page.locator(".assistant-markdown");
    await expect(answer.getByText("End of complete answer.", { exact: true })).toBeVisible({ timeout: 2000 });
    await expect(page.getByRole("button", { name: "Stop the assistant's response" })).toBeVisible();
    await expect(answer.locator(":scope > p")).toHaveCount(121);
    const plainCode = answer.locator(":scope > pre > code");
    await expect(plainCode).toHaveText("first  line\n  indented line\n", { useInnerText: false });
    expect(await plainCode.evaluate((element) => getComputedStyle(element).whiteSpace)).toBe("pre");
    const scroller = page.locator(".overflow-y-auto").filter({ has: page.locator(".chat-column-messages") });
    await expect
      .poll(() => scroller.evaluate((element) => element.scrollHeight - element.scrollTop - element.clientHeight))
      .toBeLessThanOrEqual(2);

    // A real upward wheel while the run is active must release auto-follow.
    await scroller.hover();
    await page.mouse.wheel(0, -650);
    const jump = page.getByRole("button", { name: "Scroll to the new response" });
    await expect(jump).toBeVisible();
    const continuation = `\n\n${"More analysis arrives while the reader checks history. ".repeat(60)}End of added analysis.`;
    source += continuation;
    await page.evaluate((text) => {
      const mock = (window as Window & { __E2E_CHAT_MOCK__?: { emitInferenceEvent: (event: InferenceEvent) => void } })
        .__E2E_CHAT_MOCK__;
      if (!mock) throw new Error("missing chat mock");
      mock.emitInferenceEvent({
        type: "text_delta",
        version: 1,
        run_id: "run-e2e-1",
        sequence: 1000,
        text,
      });
    }, continuation);
    await expect(answer.locator(":scope > p").last()).toHaveText(continuation.trim());
    await expect(jump).toBeVisible();
    await expect
      .poll(() => scroller.evaluate((element) => element.scrollHeight - element.scrollTop - element.clientHeight))
      .toBeGreaterThan(120);
    await jump.click();
    await expect
      .poll(() => scroller.evaluate((element) => element.scrollHeight - element.scrollTop - element.clientHeight))
      .toBeLessThanOrEqual(2);
    const streamedContent = await answer.innerText();

    await page.evaluate(() => {
      const mock = (window as Window & { __E2E_CHAT_MOCK__?: { emitInferenceEvent: (event: InferenceEvent) => void } })
        .__E2E_CHAT_MOCK__;
      if (!mock) throw new Error("missing chat mock");
      mock.emitInferenceEvent({ type: "completed", version: 1, run_id: "run-e2e-1", finish_reason: "stop" });
    });
    await expect(page.locator(".streaming-text-caret")).toHaveCount(0);
    expect(await answer.innerText()).toBe(streamedContent);

    const sends = await getChatMockInvocations(page, "chat_message_send");
    const conversationId = sends.at(-1)?.args.conversationId;
    if (typeof conversationId !== "string") throw new Error("missing submitted conversation id");
    await page.evaluate(
      async ({ id, text }) => {
        const internals = (
          window as Window & { __TAURI_INTERNALS__?: { invoke: (command: string, args: object) => Promise<unknown> } }
        ).__TAURI_INTERNALS__;
        if (!internals) throw new Error("missing native mock");
        const events: ChatProjectionEvent[] = [
          { type: "run_state_changed", version: 1, conversation_id: id, run_id: "run-e2e-1", state: "completed" },
          {
            type: "snapshot_replaced",
            version: 1,
            conversation: {
              version: 1,
              conversation_id: id,
              title: "Full analysis",
              availability: "live",
              active_run_id: null,
              messages: [
                {
                  version: 1,
                  message_id: "user-saved",
                  role: "user",
                  text: "Show the full analysis",
                  sequence: 1n,
                  tool_calls: [],
                  attachments: [],
                },
                {
                  version: 1,
                  message_id: "assistant-saved",
                  role: "assistant",
                  text,
                  sequence: 2n,
                  tool_calls: [],
                  attachments: [],
                },
              ],
            },
          },
        ];
        for (const payload of events) {
          await internals.invoke("plugin:event|emit", { event: "xfalcon://chat-projection-event", payload });
        }
      },
      { id: conversationId, text: source },
    );
    await expect(page.locator('[data-current-run-index="0"] .assistant-markdown')).toHaveCount(1);
    await expect(answer).toHaveCount(1);
    expect(await answer.innerText()).toBe(streamedContent);
    await expect(page.locator("[data-current-run-index] .message-enter")).toHaveCount(0);
    await expect
      .poll(() => scroller.evaluate((element) => element.scrollHeight - element.scrollTop - element.clientHeight))
      .toBeLessThanOrEqual(2);

    await scroller.evaluate((element) => {
      element.scrollTop = 0;
    });
    await expect(page.getByRole("button", { name: /Scroll to (latest message|the new response)/ })).toBeVisible();

    // Reopen the same source through the native conversation-view boundary.
    await installChatMocks(page, {
      bootstrapConversations: [{ conversation_id: "saved", title: "Saved analysis" }],
      conversationMessages: [
        { message_id: "user-saved", role: "user", text: "Show the full analysis", sequence: 1 },
        { message_id: "assistant-saved", role: "assistant", text: source, sequence: 2 },
      ],
    });
    await page.reload();
    await page.getByRole("option", { name: "Saved analysis" }).click();
    await expect(answer.locator(":scope > p").last()).toBeInViewport();
    expect(await answer.innerText()).toBe(streamedContent);
    await expect(answer.locator(":scope > p")).toHaveCount(122);
    await expect(answer.getByRole("table")).toHaveCount(1);
  });
}
