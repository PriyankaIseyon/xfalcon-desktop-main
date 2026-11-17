import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MessageFeedbackProvider } from "./MessageFeedback";
import { MessageRow } from "./MessageRow";

const api = vi.hoisted(() => ({ list: vi.fn(), set: vi.fn() }));
vi.mock("../../lib/native", () => ({ chatMessageFeedbackList: api.list, chatMessageFeedbackSet: api.set }));
const message = { id: "answer", role: "assistant" as const, text: "Revenue rose 15%", sequence: 2n, toolCalls: [] };
function transcript(id = "conversation") {
  return (
    <MessageFeedbackProvider key={id} conversationId={id}>
      <MessageRow conversationId={id} isLastProse message={message} onRegenerate={vi.fn()} />
    </MessageFeedbackProvider>
  );
}
afterEach(cleanup);
beforeEach(() => {
  api.list.mockReset().mockResolvedValue([]);
  api.set.mockReset();
});
describe("response feedback", () => {
  it.each([undefined, 1, -1])(
    "updates immediately and restores the exact prior selection (%s) on failure",
    async (initial) => {
      api.list.mockResolvedValue(initial === undefined ? [] : [{ message_id: "answer", rating: initial }]);
      let reject!: (reason: Error) => void;
      api.set.mockImplementation(
        () =>
          new Promise((_resolve, fail) => {
            reject = fail;
          }),
      );
      render(transcript());
      await waitFor(() =>
        expect(screen.getByRole("button", { name: initial === 1 ? "Undo upvote" : "Helpful response" })).toBeEnabled(),
      );
      const snapshot = () =>
        screen
          .getAllByRole("button")
          .filter((button) => button.hasAttribute("aria-pressed"))
          .map((button) => ({
            label: button.getAttribute("aria-label"),
            pressed: button.getAttribute("aria-pressed"),
          }));
      const before = snapshot();
      await userEvent.click(
        screen.getByRole("button", { name: initial === -1 ? "Undo downvote" : "Unhelpful response" }),
      );
      const optimistic = screen.getByRole("button", { name: initial === -1 ? "Unhelpful response" : "Undo downvote" });
      expect(optimistic).toHaveAttribute("aria-pressed", initial === -1 ? "false" : "true");
      expect(optimistic).not.toBeDisabled();
      expect(optimistic).toHaveAttribute("aria-disabled", "true");
      await userEvent.click(optimistic);
      expect(api.set).toHaveBeenCalledTimes(1);
      await act(async () => reject(new Error("offline")));
      expect(snapshot()).toEqual(before);
      expect(screen.getByRole("alert")).toHaveTextContent("Could not save");
    },
  );
  it("loads a saved vote beside regenerate, switches it and removes it through the native endpoint", async () => {
    api.list.mockResolvedValue([{ message_id: "answer", rating: 1 }]);
    api.set.mockResolvedValueOnce({ message_id: "answer", rating: -1 }).mockResolvedValueOnce(null);
    render(transcript());
    const up = await screen.findByRole("button", { name: "Undo upvote" });
    expect(up).toHaveAttribute("aria-pressed", "true");
    const regenerate = screen.getByRole("button", { name: "Regenerate response" });
    expect(regenerate.compareDocumentPosition(up) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(regenerate).not.toHaveClass("actions-slide");
    expect(screen.getByRole("button", { name: "Copy message" })).not.toHaveClass("actions-slide");
    await userEvent.click(screen.getByRole("button", { name: "Unhelpful response" }));
    expect(api.set).toHaveBeenLastCalledWith("conversation", "answer", -1);
    await userEvent.click(await screen.findByRole("button", { name: "Undo downvote" }));
    expect(api.set).toHaveBeenLastCalledWith("conversation", "answer", null);
    expect(await screen.findByRole("button", { name: "Helpful response" })).toHaveAttribute("aria-pressed", "false");
  });
  it("preserves the stored vote on write failure and allows retry", async () => {
    api.list.mockResolvedValue([{ message_id: "answer", rating: 1 }]);
    api.set.mockRejectedValueOnce(new Error("offline")).mockResolvedValueOnce({ message_id: "answer", rating: -1 });
    render(transcript());
    await screen.findByRole("button", { name: "Undo upvote" });
    await userEvent.click(screen.getByRole("button", { name: "Unhelpful response" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not save");
    expect(screen.getByRole("button", { name: "Undo upvote" })).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(screen.getByRole("button", { name: "Unhelpful response" }));
    expect(await screen.findByRole("button", { name: "Undo downvote" })).toHaveAttribute("aria-pressed", "true");
  });
  it("blocks voting until hydration succeeds and retries a failed read", async () => {
    api.list.mockRejectedValueOnce(new Error("offline")).mockResolvedValueOnce([]);
    render(transcript());
    expect(screen.getByRole("button", { name: "Helpful response" })).toBeDisabled();
    await userEvent.click(await screen.findByRole("button", { name: "Retry" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Helpful response" })).toBeEnabled());
  });
  it("ignores a previous conversation's late read", async () => {
    let resolve!: (value: unknown) => void;
    api.list.mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    const { rerender } = render(transcript("first"));
    rerender(transcript("second"));
    await waitFor(() => expect(screen.getByRole("button", { name: "Helpful response" })).toBeEnabled());
    resolve([{ message_id: "answer", rating: 1 }]);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Helpful response" })).toHaveAttribute("aria-pressed", "false"),
    );
  });
});
