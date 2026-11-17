import { expect, test } from "@playwright/test";
import { installChatMocks } from "./chat-mocks";

test.describe("chat offline and stale availability", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
  });

  test("shows offline copy when an opened conversation is stale", async ({ page }) => {
    await installChatMocks(page, {
      conversationAvailability: "stale",
      bootstrapConversations: [
        {
          conversation_id: "conv-offline-cache",
          title: "Cached tenant thread",
        },
      ],
    });
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "What can I help you with, E2E Test?" })).toBeVisible();

    await page.getByRole("option", { name: "Cached tenant thread" }).click();

    await expect(page.getByText("Offline copy — reconnect to sync")).toBeVisible();
    const composer = page.getByRole("textbox", { name: "Message xFalcon" });
    await expect(composer).toBeEnabled();
  });

  test("locks composer when workspace_status reports chat_ready false", async ({ page }) => {
    await installChatMocks(page, { chatReady: false });
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "What can I help you with, E2E Test?" })).toBeVisible();

    const composer = page.getByRole("textbox", { name: "Message xFalcon" });
    await expect(composer).toBeDisabled();
    await expect(composer).toHaveAttribute("placeholder", "\u00a0\u00a0Chat is not connected yet");
    await expect(page.getByText("No conversations yet. Start a new chat to begin.")).toBeVisible();
  });
});
