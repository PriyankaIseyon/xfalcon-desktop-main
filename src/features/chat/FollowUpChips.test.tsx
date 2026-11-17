import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChatSurface } from "./ChatSurface";
import { DEFAULT_APPEARANCE, installAppearance, resetAppearanceForTests } from "../../app/appearance";
import { AgentCapabilitiesSection } from "../settings/AgentCapabilitiesSection";

const api = vi.hoisted(() => ({
  followUps: vi.fn(),
  feedback: vi.fn(),
  appearanceGet: vi.fn(),
  appearanceSet: vi.fn(),
}));
vi.mock("../../lib/native", async (original) => ({
  ...(await original<typeof import("../../lib/native")>()),
  chatFollowUps: api.followUps,
  chatMessageFeedbackList: api.feedback,
  appearanceGet: api.appearanceGet,
  appearanceSet: api.appearanceSet,
}));
const messages = [
  { id: "question", role: "user" as const, text: "How is revenue?", sequence: 1n, toolCalls: [] },
  { id: "answer", role: "assistant" as const, text: "Revenue rose 15% in 2026.", sequence: 2n, toolCalls: [] },
];
const props = { conversationId: "conversation", messages, onSubmit: vi.fn() };
afterEach(() => {
  cleanup();
  resetAppearanceForTests();
  vi.useRealTimers();
});
beforeEach(() => {
  api.followUps.mockReset();
  api.feedback.mockResolvedValue([]);
  props.onSubmit.mockReset();
  api.appearanceGet.mockReset();
  api.appearanceSet.mockReset().mockImplementation(async (next) => next);
});
describe("Try next full surface flow", () => {
  it.each([false, true])("waits for the native preference read before generating, then honors %s", async (enabled) => {
    let resolve!: (value: unknown) => void;
    api.appearanceGet.mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    api.followUps.mockResolvedValue({ suggestions: ["Compare revenue with 2025"] });
    const installing = installAppearance();
    render(<ChatSurface {...props} />);
    expect(api.followUps).not.toHaveBeenCalled();
    await act(async () => {
      resolve({ ...DEFAULT_APPEARANCE, show_follow_ups: enabled });
      await installing;
    });
    expect(api.followUps).toHaveBeenCalledTimes(enabled ? 1 : 0);
    if (!enabled) expect(screen.queryByRole("group", { name: "Suggested follow-ups" })).not.toBeInTheDocument();
  });

  it("restores an off preference without requesting suggestions, then settings enables and disables them immediately", async () => {
    const saved = { ...DEFAULT_APPEARANCE, theme: "dark", show_reasoning: true, show_follow_ups: false };
    api.appearanceGet.mockResolvedValue(saved);
    api.followUps.mockResolvedValue({ suggestions: ["Compare revenue with 2025"] });
    await installAppearance();
    render(
      <>
        <AgentCapabilitiesSection />
        <ChatSurface {...props} />
      </>,
    );
    const toggle = screen.getByRole("switch", { name: "Try next suggestions" });
    expect(toggle).toHaveAttribute("aria-checked", "false");
    expect(api.followUps).not.toHaveBeenCalled();
    expect(screen.queryByRole("group", { name: "Suggested follow-ups" })).not.toBeInTheDocument();

    fireEvent.click(toggle);
    await screen.findByRole("button", { name: "Compare revenue with 2025" });
    expect(toggle).toHaveAttribute("aria-checked", "true");
    expect(api.appearanceSet).toHaveBeenLastCalledWith({ ...saved, show_follow_ups: true });

    fireEvent.click(toggle);
    expect(screen.queryByRole("group", { name: "Suggested follow-ups" })).not.toBeInTheDocument();
    await act(async () => {});
    expect(api.appearanceSet).toHaveBeenLastCalledWith(saved);
    expect(api.followUps).toHaveBeenCalledTimes(1);
  });

  it("uses the enabled default when loading native preferences fails", async () => {
    api.appearanceGet.mockRejectedValueOnce(new Error("unreadable"));
    api.followUps.mockResolvedValue({ suggestions: ["Compare revenue with 2025"] });
    const installing = installAppearance();
    render(<ChatSurface {...props} />);
    expect(api.followUps).not.toHaveBeenCalled();
    await act(async () => {
      await installing;
    });
    await screen.findByRole("button", { name: "Compare revenue with 2025" });
  });

  it("discards an in-flight suggestion when switched off and requests fresh suggestions when re-enabled", async () => {
    let resolve!: (value: unknown) => void;
    api.followUps
      .mockImplementationOnce(
        () =>
          new Promise((done) => {
            resolve = done;
          }),
      )
      .mockResolvedValueOnce({ suggestions: ["Fresh suggestion"] });
    render(
      <>
        <AgentCapabilitiesSection />
        <ChatSurface {...props} />
      </>,
    );
    const toggle = screen.getByRole("switch", { name: "Try next suggestions" });
    fireEvent.click(toggle);
    await act(async () => {
      resolve({ suggestions: ["Late suggestion"] });
    });
    expect(screen.queryByRole("group", { name: "Suggested follow-ups" })).not.toBeInTheDocument();
    fireEvent.click(toggle);
    await screen.findByRole("button", { name: "Fresh suggestion" });
    expect(screen.queryByRole("button", { name: "Late suggestion" })).not.toBeInTheDocument();
  });

  it("requests contextual prompts and sends the chosen text through the normal submit handler", async () => {
    api.followUps.mockResolvedValue({ suggestions: ["Break revenue down by region", "Compare revenue with 2025"] });
    render(<ChatSurface {...props} />);
    fireEvent.click(await screen.findByRole("button", { name: "Break revenue down by region" }));
    expect(api.followUps).toHaveBeenCalledWith("conversation", messages[1].text, messages[0].text);
    expect(props.onSubmit).toHaveBeenCalledExactlyOnceWith("Break revenue down by region");
    fireEvent.click(screen.getByRole("button", { name: "Dismiss follow-up suggestions" }));
    expect(screen.queryByRole("group", { name: "Suggested follow-ups" })).not.toBeInTheDocument();
  });
  it.each([
    { streaming: true },
    { submitting: true },
    { revisionPending: true },
    { thinking: true },
    { availabilityStale: true },
  ])("does not generate while unavailable: %j", async (state) => {
    render(<ChatSurface {...props} {...state} />);
    expect(screen.queryByRole("group", { name: "Suggested follow-ups" })).not.toBeInTheDocument();
    expect(api.followUps).not.toHaveBeenCalled();
  });
  it("does not offer old prompts after the next user turn or on a synthetic answer", () => {
    const { rerender } = render(
      <ChatSurface {...props} messages={[...messages, { ...messages[0], id: "new", sequence: 3n }]} />,
    );
    expect(api.followUps).not.toHaveBeenCalled();
    rerender(<ChatSurface {...props} messages={[messages[0], { ...messages[1], id: "stream-run" }]} />);
    expect(api.followUps).not.toHaveBeenCalled();
  });
  it("uses contextual local prompts on provider failure", async () => {
    api.followUps.mockRejectedValue(new Error("provider unavailable"));
    render(<ChatSurface {...props} />);
    fireEvent.click(await screen.findByRole("button", { name: "What is driving this change?" }));
    expect(props.onSubmit).toHaveBeenCalledWith("What is driving this change?");
  });
  it("offers BI starters when an acknowledgment has no useful model suggestions", async () => {
    api.followUps.mockResolvedValue({ suggestions: [] });
    render(
      <ChatSurface
        {...props}
        messages={[
          { ...messages[0], text: "Thank you" },
          { ...messages[1], text: "You're welcome." },
        ]}
      />,
    );
    const prompt = "Help me plan a KPI dashboard for my business";
    fireEvent.click(await screen.findByRole("button", { name: prompt }));
    expect(props.onSubmit).toHaveBeenCalledWith(prompt);
    expect(
      screen.getByRole("button", { name: "Help me define data quality checks for an ETL pipeline" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /translate|concise version|alternatives/i })).not.toBeInTheDocument();
  });
  it("discards late suggestions after navigation", async () => {
    let resolve!: (value: unknown) => void;
    api.followUps
      .mockImplementationOnce(
        () =>
          new Promise((done) => {
            resolve = done;
          }),
      )
      .mockResolvedValueOnce({ suggestions: ["Explore the second answer"] });
    const { rerender } = render(<ChatSurface {...props} />);
    rerender(<ChatSurface {...props} conversationId="second" />);
    await screen.findByRole("button", { name: "Explore the second answer" });
    await act(async () => {
      resolve({ suggestions: ["Old private prompt"] });
    });
    expect(screen.queryByRole("button", { name: "Old private prompt" })).not.toBeInTheDocument();
  });
  it("commits fallback once and never swaps it for a late model answer", async () => {
    vi.useFakeTimers();
    let resolve!: (value: unknown) => void;
    api.followUps.mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    render(<ChatSurface {...props} />);
    await act(async () => {
      vi.advanceTimersByTime(7500);
    });
    expect(screen.getByRole("button", { name: "What is driving this change?" })).toBeInTheDocument();
    await act(async () => {
      resolve({ suggestions: ["Late answer"] });
    });
    expect(screen.queryByRole("button", { name: "Late answer" })).not.toBeInTheDocument();
  });
});
