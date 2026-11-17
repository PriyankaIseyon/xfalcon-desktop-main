import { expect, test } from "@playwright/test";
import { getChatMockInvocations, installChatMocks } from "./chat-mocks";

test.describe("chat slice exit gate (Path B mocks)", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
  });

  test("shows optimistic user message after send with pending mock invoke", async ({ page }) => {
    await installChatMocks(page, { pendingSend: true });
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "What can I help you with, E2E Test?" })).toBeVisible();

    const composer = page.getByRole("textbox", { name: "Message xFalcon" });
    await expect(composer).toBeEnabled();
    await composer.fill("Slice gate probe message");
    await page.getByRole("button", { name: "Send message" }).click();

    await expect(page.getByRole("region", { name: "Chat" }).getByText("Slice gate probe message")).toBeVisible();
  });

  for (const reducedMotion of ["no-preference", "reduce"] as const) {
    test(`keeps a quiet streaming caret beside wrapped text (${reducedMotion})`, async ({ page }) => {
      await page.emulateMedia({ reducedMotion });
      await page.setViewportSize({ width: 520, height: 700 });
      await installChatMocks(page, { emitRunningOnSend: true, pendingSend: true });
      await page.goto("/");

      const composer = page.getByRole("textbox", { name: "Message xFalcon" });
      await composer.fill("Explain the result");
      await page.getByRole("button", { name: "Send message" }).click();

      const streamedText =
        "The final marker must stay beside the last painted character while this answer wraps across several narrow lines.";
      await page.evaluate((text) => {
        const mock = (
          window as Window & {
            __E2E_CHAT_MOCK__?: {
              emitInferenceEvent: (event: {
                type: "text_delta";
                version: 1;
                run_id: string;
                sequence: number;
                text: string;
              }) => void;
            };
          }
        ).__E2E_CHAT_MOCK__;
        if (!mock) throw new Error("missing chat mock");
        mock.emitInferenceEvent({ type: "text_delta", version: 1, run_id: "run-e2e-1", sequence: 1, text });
      }, streamedText);

      const caret = page.locator(".streaming-text-caret[data-positioned='true']");
      await expect(caret).toBeVisible();
      await expect(page.locator(".assistant-markdown")).toContainText(streamedText);

      const geometry = await page.locator(".assistant-markdown").evaluate((root) => {
        const marker = root.querySelector<HTMLElement>(".streaming-text-caret");
        if (!marker) throw new Error("missing streaming caret");
        const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
        const textNodes: Text[] = [];
        let current = walker.nextNode();
        while (current) {
          if (current instanceof Text && current.data.trim().length > 0) textNodes.push(current);
          current = walker.nextNode();
        }
        const finalText = textNodes.at(-1);
        if (!finalText) throw new Error("missing streamed text node");
        const end = finalText.data.search(/\s*$/u);
        const range = document.createRange();
        range.setStart(finalText, end - 1);
        range.setEnd(finalText, end);
        const characterRect = Array.from(range.getClientRects()).at(-1);
        if (!characterRect) throw new Error("missing final character rect");
        const markerRect = marker.getBoundingClientRect();
        const rootStyle = getComputedStyle(root);
        return {
          horizontalDelta: Math.abs(markerRect.left - characterRect.right),
          verticalDelta: Math.abs(markerRect.top - characterRect.top),
          heightDelta: Math.abs(markerRect.height - characterRect.height),
          wrapped: root.getBoundingClientRect().height > Number.parseFloat(rootStyle.lineHeight) * 2,
        };
      });

      expect(geometry.wrapped).toBe(true);
      expect(geometry.horizontalDelta).toBeLessThan(0.75);
      expect(geometry.verticalDelta).toBeLessThan(0.75);
      expect(geometry.heightDelta).toBeLessThan(0.75);

      const pulse = await caret.evaluate((marker) => {
        const animations = marker.getAnimations();
        const animation = animations[0];
        if (animation?.effect) {
          animation.pause();
          animation.currentTime = Number(animation.effect.getComputedTiming().duration) / 2;
        }
        return {
          animations: animations.length,
          opacity: Number(getComputedStyle(marker).opacity),
          transition: getComputedStyle(marker).transitionDuration,
        };
      });
      if (reducedMotion === "reduce") {
        expect(pulse.animations).toBe(0);
        expect(pulse.opacity).toBe(1);
      } else {
        expect(pulse.animations).toBe(1);
        expect(pulse.opacity).toBeGreaterThan(0);
        expect(pulse.opacity).toBeLessThan(1);
        expect(pulse.transition).toBe("0s");
      }
    });
  }

  test("disables composer when workspace chat is not ready", async ({ page }) => {
    await installChatMocks(page, { chatReady: false });
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "What can I help you with, E2E Test?" })).toBeVisible();

    const composer = page.getByRole("textbox", { name: "Message xFalcon" });
    await expect(composer).toBeDisabled();
    await expect(composer).toHaveAttribute("placeholder", "\u00a0\u00a0Chat is not connected yet");
  });

  test("New chat clears optimistic messages and returns to empty state", async ({ page }) => {
    await installChatMocks(page, { pendingSend: true });
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "What can I help you with, E2E Test?" })).toBeVisible();

    const composer = page.getByRole("textbox", { name: "Message xFalcon" });
    await expect(composer).toBeEnabled();
    await composer.fill("Before new chat");
    await page.getByRole("button", { name: "Send message" }).click();
    const chat = page.getByRole("region", { name: "Chat" });
    await expect(chat.getByText("Before new chat")).toBeVisible();

    await page.getByRole("button", { name: "New chat" }).click();

    await expect(page.getByRole("heading", { name: "What can I help you with, E2E Test?" })).toBeVisible();
    await expect(chat.getByText("Before new chat")).not.toBeVisible();
    await expect(composer).toHaveValue("");

    const creates = await getChatMockInvocations(page, "chat_conversation_create");
    expect(creates.length).toBeGreaterThanOrEqual(1);
    expect(creates[0]?.args).toEqual({ openingMessage: "Before new chat" });
  });

  test("chat_conversation_create mock returns a live empty conversation view", async ({ page }) => {
    await installChatMocks(page);
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "What can I help you with, E2E Test?" })).toBeVisible();

    const view = await page.evaluate(async () => {
      const internals = (
        window as Window & { __TAURI_INTERNALS__?: { invoke: (cmd: string, args?: object) => Promise<unknown> } }
      ).__TAURI_INTERNALS__;
      if (!internals) {
        throw new Error("missing Tauri mock");
      }
      return internals.invoke("chat_conversation_create", { openingMessage: "Slice create probe" });
    });

    expect(view).toMatchObject({
      version: 1,
      title: "Slice create probe",
      availability: "live",
      messages: [],
      active_run_id: null,
    });
    expect(view).toHaveProperty("conversation_id");

    const creates = await getChatMockInvocations(page, "chat_conversation_create");
    expect(creates).toHaveLength(1);
    expect(creates[0]?.args).toEqual({ openingMessage: "Slice create probe" });
  });

  test("invokes chat_run_cancel from the composer's stop control", async ({ page }) => {
    // Pending send + synthetic running run event sets activeRunId/runState (no direct store setState in e2e).
    await installChatMocks(page, { pendingSend: true, emitRunningOnSend: true });
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "What can I help you with, E2E Test?" })).toBeVisible();

    const composer = page.getByRole("textbox", { name: "Message xFalcon" });
    await expect(composer).toBeEnabled();
    await composer.fill("Cancel gate probe");
    await page.getByRole("button", { name: "Send message" }).click();

    // Spec chat-composer: while the run is in flight the circular send
    // control IS the stop control — no separate pill anywhere.
    const cancel = page.getByRole("button", { name: "Stop the assistant's response" });
    await expect(cancel).toBeVisible();
    await expect(cancel).toHaveCSS("background-color", "rgb(245, 124, 118)");
    await expect(cancel.locator(".composer-stop__glyph")).toHaveCSS("background-color", "rgb(51, 51, 51)");
    await expect(cancel.locator(".composer-stop__glyph")).toHaveCSS("width", "10px");
    await expect(cancel.locator(".composer-stop__glyph")).toHaveCSS("border-radius", "3px");
    await cancel.click();

    const cancels = await getChatMockInvocations(page, "chat_run_cancel");
    expect(cancels).toHaveLength(1);
    expect(cancels[0]?.args.runId).toMatch(/^run-e2e-/);
  });

  test("invokes chat_run_retry when recovery notice retry is clicked", async ({ page }) => {
    await installChatMocks(page, { emitRunningOnSend: true, emitInterruptedAfterRunning: true });
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "What can I help you with, E2E Test?" })).toBeVisible();

    const composer = page.getByRole("textbox", { name: "Message xFalcon" });
    await expect(composer).toBeEnabled();
    await composer.fill("Retry gate probe");
    await page.getByRole("button", { name: "Send message" }).click();

    await expect(page.getByText("interrupted", { exact: false })).toBeVisible();
    const retry = page.getByRole("button", { name: "Retry run" });
    await expect(retry).toBeVisible();
    await retry.click();

    const retries = await getChatMockInvocations(page, "chat_run_retry");
    expect(retries).toHaveLength(1);
    expect(retries[0]?.args.runId).toMatch(/^run-e2e-/);
  });

  test("restart surfaces bootstrap recovery and scopes run events to the selected conversation", async ({ page }) => {
    const notice = "This response was interrupted before it finished. You can retry when ready.";
    const conversationId = "conv-recovered-mid-stream";
    await installChatMocks(page, { emitRunningOnSend: true, pendingSend: true });
    await page.goto("/");
    await page.getByRole("textbox", { name: "Message xFalcon" }).fill("Mid-stream gate four");
    await page.getByRole("button", { name: "Send message" }).click();
    const chat = page.getByRole("region", { name: "Chat" });
    await expect(chat.getByText("Mid-stream gate four")).toBeVisible();

    await installChatMocks(page, {
      recoveryNotice: notice,
      bootstrapConversations: [
        {
          conversation_id: conversationId,
          title: "Recovered analysis",
        },
      ],
      conversationMessages: [
        {
          message_id: "message-recovered-user",
          role: "user",
          text: "Mid-stream gate four",
          sequence: 1,
        },
        {
          message_id: "message-recovered-assistant",
          role: "assistant",
          text: "Partial recovered answer",
          sequence: 2,
        },
      ],
      conversationActiveRunId: "run-recovered-mid-stream",
    });
    await page.reload();
    await expect(page.getByRole("heading", { name: "What can I help you with, E2E Test?" })).toBeVisible();
    await expect(page.getByText(/interrupted before it finished/i)).toBeVisible();
    await expect(page.getByRole("button", { name: "Retry run" })).not.toBeVisible();
    await page.getByRole("option", { name: "Recovered analysis" }).click();
    await expect(chat.getByText("Mid-stream gate four")).toHaveCount(1);
    await expect(chat.getByText("Partial recovered answer")).toHaveCount(1);
    await expect(page.getByText(/interrupted before it finished/i)).toBeVisible();
    await expect(page.getByRole("button", { name: "Retry run" })).toBeVisible();
    await page.getByRole("button", { name: "Dismiss" }).click();
    await expect(page.getByText(/interrupted before it finished/i)).not.toBeVisible();
    await expect(page.getByRole("button", { name: "Retry run" })).not.toBeVisible();

    const emitInterrupted = async (id: string) => {
      await page.evaluate((selectedConversationId) => {
        const mock = (
          window as Window & {
            __E2E_CHAT_MOCK__?: {
              emitRunProjection: (conversationId: string, runId: string, state: string) => void;
            };
          }
        ).__E2E_CHAT_MOCK__;
        if (!mock) throw new Error("missing chat mock");
        mock.emitRunProjection(selectedConversationId, "run-recovered-mid-stream", "interrupted");
      }, id);
    };
    await emitInterrupted("another-conversation");
    await expect(page.getByRole("button", { name: "Retry run" })).not.toBeVisible();
    await emitInterrupted(conversationId);
    await expect(page.getByText(/interrupted before it finished/i)).toBeVisible();
    await expect(page.getByRole("button", { name: "Retry run" })).toBeVisible();

    await page.getByRole("button", { name: "New chat" }).click();
    await expect(page.getByRole("heading", { name: "What can I help you with, E2E Test?" })).toBeVisible();
    await emitInterrupted(conversationId);
    await expect(page.getByText(/interrupted before it finished/i)).not.toBeVisible();
    await expect(page.getByRole("button", { name: "Retry run" })).not.toBeVisible();
  });
});
