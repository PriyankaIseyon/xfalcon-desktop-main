import { expect, test } from "@playwright/test";
import { getChatMockInvocations, installChatMocks } from "../e2e/chat-mocks";

for (const theme of ["light", "dark"] as const) {
  test(`response actions and next prompts work without hover in ${theme}`, async ({ page }) => {
    await page.setViewportSize({ width: 1000, height: 800 });
    await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
    await installChatMocks(page, {
      appearanceTheme: theme,
      bootstrapConversations: [{ conversation_id: "conversation-actions", title: "Revenue review" }],
      conversationMessages: [
        { message_id: "question", role: "user", text: "How is revenue?", sequence: 1 },
        { message_id: "answer", role: "assistant", text: "Revenue rose 15%. Europe led the increase.", sequence: 2 },
      ],
      feedbackRatings: { answer: 1 },
      followUpSuggestions: ["Break Europe revenue down by country", "Compare revenue with last year"],
    });
    await page.goto("/");
    await page.getByRole("option", { name: "Revenue review", exact: true }).click();
    const regenerate = page.getByRole("button", { name: "Regenerate response", exact: true });
    await expect(regenerate).toBeVisible();
    await page.mouse.move(0, 0);
    for (const action of [regenerate, page.getByRole("button", { name: "Copy message", exact: true }).last()]) {
      await expect(action).toHaveCSS("opacity", "1");
    }
    await expect(page.getByRole("button", { name: "Undo upvote", exact: true })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(page.getByRole("button", { name: "Undo upvote", exact: true })).toHaveCSS(
      "color",
      theme === "light" ? "rgb(19, 122, 75)" : "rgb(98, 217, 138)",
    );
    await page.getByRole("button", { name: "Unhelpful response", exact: true }).click();
    await expect(page.getByRole("button", { name: "Undo downvote", exact: true })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(page.getByRole("button", { name: "Undo downvote", exact: true })).toHaveCSS(
      "color",
      theme === "light" ? "rgb(192, 54, 44)" : "rgb(234, 88, 12)",
    );
    await page.getByRole("button", { name: "Undo downvote", exact: true }).click();
    expect((await getChatMockInvocations(page, "chat_message_feedback_set")).map((entry) => entry.args.rating)).toEqual(
      [-1, null],
    );
    const prompt = "Break Europe revenue down by country";
    await expect(page.getByRole("button", { name: prompt, exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: prompt, exact: true })).toHaveCSS(
      "border-color",
      theme === "light" ? "rgb(24, 114, 136)" : "rgb(94, 216, 236)",
    );
    await expect(page.getByRole("button", { name: prompt, exact: true })).toHaveCSS("border-width", "1px");
    const chipsBounds = await page.getByRole("group", { name: "Suggested follow-ups" }).boundingBox();
    const actionsBounds = await regenerate.boundingBox();
    expect(chipsBounds!.y).toBeGreaterThan(actionsBounds!.y + actionsBounds!.height);
    await page.screenshot({ path: `test-results/response-actions-${theme}.png` });
    await page.getByRole("button", { name: prompt, exact: true }).click();
    await expect.poll(async () => (await getChatMockInvocations(page, "chat_message_send")).length).toBe(1);
    expect((await getChatMockInvocations(page, "chat_message_send"))[0].args).toMatchObject({ prompt });
    await expect(page.getByRole("group", { name: "Suggested follow-ups" })).toHaveCount(0);
  });
}
