import { expect, test } from "@playwright/test";
import { installChatMocks } from "./chat-mocks";

test("opens long conversations at the final answer and preserves deliberate history scrolling", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await installChatMocks(page, {
    bootstrapConversations: [
      { conversation_id: "history-a", title: "First long conversation" },
      { conversation_id: "history-b", title: "Second long conversation" },
    ],
    conversationMessages: Array.from({ length: 80 }, (_, index) => ({
      message_id: `history-${index}`,
      role: index % 2 === 0 ? "user" : "assistant",
      text:
        index === 79
          ? `${"A detailed final answer with many paragraphs.\n\n".repeat(70)}End of final answer.`
          : `Turn ${index}. ${"Variable length history. ".repeat((index % 7) * 20 + 1)}`,
      sequence: index + 1,
    })),
  });
  await page.goto("/");
  await page.getByRole("option", { name: "First long conversation" }).click();
  const scroller = page
    .getByRole("region", { name: "Chat" })
    .locator(".overflow-y-auto")
    .filter({
      has: page.locator(".chat-column-messages"),
    });
  const distanceFromBottom = () =>
    scroller.evaluate((element) => element.scrollHeight - element.scrollTop - element.clientHeight);
  await expect.poll(distanceFromBottom).toBeLessThanOrEqual(2);
  await expect(page.getByText("End of final answer.", { exact: true })).toBeInViewport();

  await scroller.evaluate((element) => {
    element.scrollTop = 0;
  });
  await expect(page.getByRole("button", { name: "Scroll to latest message" })).toBeVisible();
  // Remeasuring wrapped history must not pull someone reading it to the end.
  await page.setViewportSize({ width: 1000, height: 800 });
  await expect.poll(distanceFromBottom).toBeGreaterThan(500);

  await page.getByRole("option", { name: "Second long conversation" }).click();
  await expect.poll(distanceFromBottom).toBeLessThanOrEqual(2);
  await expect(page.getByText("End of final answer.", { exact: true })).toBeInViewport();
  await page.setViewportSize({ width: 1200, height: 900 });
  await expect.poll(distanceFromBottom).toBeLessThanOrEqual(2);
});

test("historical rows shrink with reading size without gaps or overlapping text", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await installChatMocks(page, {
    bootstrapConversations: [{ conversation_id: "history", title: "Reading size history" }],
    conversationMessages: Array.from({ length: 40 }, (_, index) => ({
      message_id: `history-${index}`,
      role: index % 2 === 0 ? "user" : "assistant",
      text: `Turn ${index}. ${"A variable length paragraph for reading and layout. ".repeat((index % 4) * 8 + 1)}`,
      sequence: index + 1,
    })),
  });
  await page.goto("/");
  await page.getByRole("option", { name: "Reading size history" }).click();
  const scroller = page.locator(".overflow-y-auto").filter({ has: page.locator(".chat-column-messages") });
  await scroller.evaluate((element) => {
    element.scrollTop = 0;
  });
  // This is the document attribute written by applyAppearance. Keep the
  // transcript mounted so the test exercises existing measurements.
  await page.evaluate(() => {
    document.documentElement.dataset.fontSize = "large";
  });
  const firstAnswer = page.locator('[data-index="1"]');
  await expect(firstAnswer).toBeVisible();
  const largestRowGap = () =>
    page.locator(".chat-column-messages ul > li").evaluateAll((rows) => {
      let largestError = 0;
      for (let index = 1; index < rows.length; index += 1) {
        const previous = rows[index - 1].getBoundingClientRect();
        const current = rows[index].getBoundingClientRect();
        largestError = Math.max(largestError, Math.abs(current.top - previous.bottom));
      }
      return largestError;
    });
  await expect.poll(largestRowGap).toBeLessThanOrEqual(2);
  const expandedHeight = await firstAnswer.evaluate((element) => element.getBoundingClientRect().height);
  await page.evaluate(() => {
    document.documentElement.dataset.fontSize = "small";
  });
  await expect
    .poll(() => firstAnswer.evaluate((element) => element.getBoundingClientRect().height))
    .toBeLessThan(expandedHeight);
  await expect.poll(largestRowGap).toBeLessThanOrEqual(2);
  // Rows measured before the font change refresh when they remount, too.
  await scroller.evaluate((element) => {
    element.scrollTop = element.scrollHeight * 0.7;
  });
  await expect(page.locator('[data-index="30"]')).toBeAttached();
  await expect.poll(largestRowGap).toBeLessThanOrEqual(2);
});
