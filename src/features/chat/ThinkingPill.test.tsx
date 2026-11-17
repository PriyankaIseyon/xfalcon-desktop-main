import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { applyInferenceForTests, resetChatStore, setChatStoreForTests } from "./chatStore";
import { ThinkingPill } from "./ThinkingPill";

afterEach(() => {
  cleanup();
  resetChatStore();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

/** Holds one phrase, then lets the next one land. */
function holdOnePhrase() {
  act(() => {
    vi.advanceTimersByTime(2000);
  });
}

describe("ThinkingPill", () => {
  it("leads with Thinking and cycles a fixed set of phrases", () => {
    vi.useFakeTimers();
    render(<ThinkingPill />);

    // The first phrase is what a submitted turn shows before its run exists.
    expect(screen.getByText("Thinking")).toBeInTheDocument();

    holdOnePhrase();
    expect(screen.getByText("Working")).toBeInTheDocument();
    expect(screen.queryByText("Thinking")).toBeNull();

    holdOnePhrase();
    expect(screen.getByText("Reasoning")).toBeInTheDocument();
  });

  it("returns to the first phrase rather than running out of them", () => {
    vi.useFakeTimers();
    render(<ThinkingPill />);

    // Five phrases: five holds bring the cycle back to its start, so a run
    // that waits a long time never lands on an empty label.
    for (let hold = 0; hold < 5; hold += 1) {
      holdOnePhrase();
    }

    expect(screen.getByText("Thinking")).toBeInTheDocument();
  });

  it("holds one phrase for a reader who asked for no motion", () => {
    vi.useFakeTimers();
    vi.stubGlobal(
      "matchMedia",
      vi.fn((query: string) => ({ matches: query.includes("prefers-reduced-motion"), media: query })),
    );

    render(<ThinkingPill />);
    holdOnePhrase();

    expect(screen.getByText("Thinking")).toBeInTheDocument();
  });

  it("says one stable thing to a screen reader instead of five", () => {
    vi.useFakeTimers();
    render(<ThinkingPill />);

    // The rotation is decorative: announcing it would repeat the same message
    // every two seconds.
    const status = screen.getByRole("status");
    expect(status).toHaveTextContent("Working on the request");
    expect(screen.getByText("Thinking").closest("[aria-hidden]")).not.toBeNull();

    holdOnePhrase();
    expect(status).toHaveTextContent("Working on the request");
  });

  it("shows the blue spinner without a background tile", () => {
    const { container } = render(<ThinkingPill />);
    const spinner = container.querySelector(".tool-activity-shell__spinner");

    expect(spinner).not.toBeNull();
    expect(spinner?.closest(".tool-activity-shell__leading")).toHaveClass("tool-activity-shell__leading--bare");
  });

  it("says the history is being summarized while its run does that, and only then", () => {
    vi.useFakeTimers();
    setChatStoreForTests({ activeRunId: "run-1", runState: "running" });
    render(<ThinkingPill />);
    expect(screen.getByText("Thinking")).toBeInTheDocument();

    act(() => {
      applyInferenceForTests({ type: "context_compaction", version: 1, run_id: "run-1", active: true });
    });
    expect(screen.getByText("Summarizing")).toBeInTheDocument();
    // Rotation keeps running underneath, and must not replace the notice.
    holdOnePhrase();
    expect(screen.getByText("Summarizing")).toBeInTheDocument();

    act(() => {
      applyInferenceForTests({ type: "context_compaction", version: 1, run_id: "run-1", active: false });
    });
    expect(screen.queryByText("Summarizing")).toBeNull();
  });

  it("ignores a compaction notice that belongs to another run", () => {
    setChatStoreForTests({ activeRunId: "run-1", runState: "running" });
    render(<ThinkingPill />);

    act(() => {
      applyInferenceForTests({ type: "context_compaction", version: 1, run_id: "run-2", active: true });
    });

    expect(screen.queryByText("Summarizing")).toBeNull();
  });
});
