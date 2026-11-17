import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { GREETING_MS } from "../delight/greeting";
import { EmptyConversation } from "./EmptyConversation";

const claimMock = vi.hoisted(() => vi.fn());

vi.mock("../../lib/native", () => ({ delightGreetingClaim: claimMock }));

async function settle() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("EmptyConversation daily greeting", () => {
  beforeEach(() => {
    claimMock.mockReset();
    vi.useFakeTimers();
  });
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("opens the day with a greeting above the heading, then removes it", async () => {
    claimMock.mockResolvedValue(true);
    render(<EmptyConversation canSuggest greet onSuggest={vi.fn()} viewerName="Jeevan" />);
    await settle();
    const greeting = screen.getByRole("status");
    expect(greeting.textContent).toBeTruthy();
    // Above the heading, and the heading is untouched.
    const heading = screen.getByRole("heading", { name: "What can I help you with, Jeevan?" });
    expect(greeting.compareDocumentPosition(heading) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    act(() => void vi.advanceTimersByTime(GREETING_MS));
    expect(screen.queryByRole("status")).toBeNull();
    expect(heading).toBeInTheDocument();
  });

  it("has no greeting when the day was already greeted", async () => {
    claimMock.mockResolvedValue(false);
    render(<EmptyConversation canSuggest greet onSuggest={vi.fn()} />);
    await settle();
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("does not use up the day's greeting unless it is the new-chat screen", async () => {
    claimMock.mockResolvedValue(true);
    render(<EmptyConversation canSuggest onSuggest={vi.fn()} />);
    await settle();
    expect(claimMock).not.toHaveBeenCalled();
    expect(screen.queryByRole("status")).toBeNull();
  });
});
