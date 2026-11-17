import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_APPEARANCE, chooseAppearance, resetAppearanceForTests } from "../../app/appearance";
import { ChatSurface } from "./ChatSurface";

// The shipped default withholds payloads and step summaries
// (`./activityDisclosure`). The code behind them is intact and one flag away, so
// this suite forces disclosure ON and keeps proving the properties that matter
// if it is ever turned back on — hostile content stays inert, paths stay
// literal, nested panels still settle their row. What the DEFAULT draws is
// pinned separately, in `activityDisclosure.test.tsx`. Reasoning display is a
// preference explicitly enabled for reasoning cases; one turns it off through
// the store.
vi.mock("./activityDisclosure", () => ({
  SHOW_TOOL_PAYLOADS: true,
  SHOW_TOOL_STEP_SUMMARY: true,
}));

async function showReasoning() {
  // Exercise the real appearance store. No native host is available in jsdom;
  // the choice is applied before the persistence leg rejects.
  await chooseAppearance({ ...DEFAULT_APPEARANCE, show_reasoning: true }, { dataset: {} }).catch(() => {});
}

afterEach(cleanup);
afterEach(resetAppearanceForTests);
afterEach(() => vi.unstubAllGlobals());

describe("ChatSurface", () => {
  it("renders a calm, honest chat-first empty state", () => {
    render(<ChatSurface />);

    expect(screen.getByRole("region", { name: "Chat" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "What can I help you with?" })).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Message xFalcon" })).toBeInTheDocument();
    expect(screen.queryByText(/progress/i)).not.toBeInTheDocument();
  });

  it("does not ask a provider-ready workspace to add a provider key", () => {
    render(<ChatSurface onOpenSettings={vi.fn()} onSubmit={vi.fn()} />);

    expect(screen.queryByRole("button", { name: /provider API key in Settings/i })).not.toBeInTheDocument();
  });

  it("gives a provider-unready workspace the settings path", () => {
    render(<ChatSurface disabled onOpenSettings={vi.fn()} />);

    expect(screen.getByRole("button", { name: /provider API key in Settings/i })).toBeInTheDocument();
  });

  it("shows offline copy when availability is stale", () => {
    render(<ChatSurface availabilityStale />);
    expect(screen.getByText("Offline copy — reconnect to sync")).toBeInTheDocument();
  });

  it("forwards the active project through to the composer as compact context", () => {
    render(<ChatSurface activeProjectName="Research" />);
    expect(screen.getByText("Research")).toBeInTheDocument();
  });

  it("keeps an unavailable project chat readable while explaining that local tools are off", () => {
    render(<ChatSurface activeProjectName="Research" projectUnavailable onSubmit={vi.fn()} />);

    expect(screen.getByRole("status")).toHaveTextContent("Project folder unavailable");
    expect(screen.getByRole("status")).toHaveTextContent("local project tools are off");
    expect(screen.getByRole("textbox")).toBeEnabled();
  });

  it("reports one death as one card, preferring the account that names the cause", () => {
    // A dead run raises both an error and a recovery notice. Two cards told
    // the reader the same thing twice, in two voices.
    render(
      <ChatSurface
        error="A tool took too long to answer, so the step was abandoned. You can retry."
        messages={[{ id: "m-1", role: "user", text: "hi", sequence: BigInt(1), toolCalls: [] }]}
        recoveryNotice="This response was interrupted before it finished. You can retry when ready."
      />,
    );

    expect(screen.getAllByRole("alert")).toHaveLength(1);
    expect(screen.queryByText(/interrupted before it finished/)).not.toBeInTheDocument();
    expect(screen.getByText(/A tool took too long to answer/)).toBeInTheDocument();
  });

  it("falls back to the recovery copy when a run died without an account", () => {
    render(
      <ChatSurface
        messages={[{ id: "m-1", role: "user", text: "hi", sequence: BigInt(1), toolCalls: [] }]}
        recoveryNotice="This response was interrupted before it finished."
      />,
    );

    expect(screen.getByRole("status")).toBeInTheDocument();
    expect(screen.getByText("This response was interrupted before it finished.")).toBeInTheDocument();
  });
});

describe("ChatSurface reasoning (spec: chat-reasoning-panel)", () => {
  beforeEach(showReasoning);
  const message = { id: "m-1", role: "user" as const, text: "hi", sequence: BigInt(1), toolCalls: [] };
  const disclosures = () =>
    screen.queryAllByRole("button", { name: /Working|Thinking|Thought process|tools? used|tool calls?/ });

  it("puts reasoning inside the same pill the waiting state used", () => {
    render(
      <ChatSurface
        messages={[message]}
        reasoning={[{ text: "weighing the options", afterCallCount: 0 }]}
        runActive
        thinking
      />,
    );
    // One control, still wearing the waiting phrase. It used to be replaced by
    // a second disclosure named "Thought process".
    expect(disclosures()).toHaveLength(1);
    fireEvent.click(disclosures()[0]);
    expect(screen.getByText("weighing the options")).toBeInTheDocument();
  });

  it("shows the pill and no disclosure for a run that streamed neither", () => {
    render(<ChatSurface messages={[message]} runActive thinking />);
    expect(screen.getByText("Thinking")).toBeInTheDocument();
    expect(disclosures()).toHaveLength(0);
  });

  it("keeps the pill when reasoning arrives but is not being drawn", async () => {
    // The defect: the pill was suppressed by `reasoning.length > 0` while the
    // surface drawing that reasoning had its own, different condition. A run
    // that reasoned with reasoning withheld then showed an auto-expanded card
    // with an empty body and no indicator at all.
    const target = { dataset: {} as Record<string, string> };
    await chooseAppearance({ ...DEFAULT_APPEARANCE, show_reasoning: false }, target).catch(() => {});
    render(
      <ChatSurface
        messages={[message]}
        reasoning={[{ text: "weighing the options", afterCallCount: 0 }]}
        runActive
        thinking
      />,
    );
    expect(screen.getByText("Thinking")).toBeInTheDocument();
    expect(disclosures()).toHaveLength(0);
    expect(screen.queryByText("weighing the options")).toBeNull();
  });

  // The live timeline is gone once the run settles, and the reasoning it held is
  // the only copy there has ever been — the transcript takes it over, collapsed
  // but reopenable.
  it("hands a settled run's reasoning to the transcript", () => {
    render(
      <ChatSurface activity={[]} messages={[message]} reasoning={[{ text: "no tools needed", afterCallCount: 0 }]} />,
    );
    expect(screen.getByRole("button", { name: /Thought process/ })).toBeInTheDocument();
  });

  // Unified activity card holds thought process reasoning followed by tool rows.
  it("orders reasoning and tool rows in unified activity card", () => {
    const { container } = render(
      <ChatSurface
        activity={[
          { type: "requested", version: 1, invocation_id: "inv-1", run_id: "run-1", tool_id: "project.read_file" },
        ]}
        error="It failed."
        messages={[message]}
        reasoning={[
          { text: "before the call", afterCallCount: 0 },
          { text: "after the result", afterCallCount: 1 },
        ]}
        runActive
      />,
    );
    // Reasoning and tool activity share one active pill, closed until opened.
    const liveButtons = screen.getAllByRole("button", { name: /Thinking/ });
    expect(liveButtons).toHaveLength(1);
    fireEvent.click(liveButtons[0]);
    const text = container.textContent ?? "";
    expect(text.indexOf("before the call")).toBeLessThan(text.indexOf("project.read_file"));
    expect(text.indexOf("project.read_file")).toBeLessThan(text.indexOf("after the result"));
    expect(text.indexOf("after the result")).toBeLessThan(text.indexOf("It failed."));
  });

  it("keeps one pill when the durable projection overlaps the live run", () => {
    render(
      <ChatSurface
        activity={[
          { type: "requested", version: 1, invocation_id: "inv-1", run_id: "run-1", tool_id: "project.read_file" },
        ]}
        messages={[
          message,
          {
            id: "m-tool",
            role: "assistant",
            text: "",
            sequence: BigInt(2),
            toolCalls: [
              {
                call_id: "inv-1",
                name: "project.read_file",
                status: "answered",
                arguments: null,
                arguments_truncated: false,
                result: null,
                result_truncated: false,
                step: null,
              },
            ],
          },
        ]}
        reasoning={[{ text: "checking the file first", afterCallCount: 0 }]}
        runActive
      />,
    );

    expect(disclosures()).toHaveLength(1);
    fireEvent.click(disclosures()[0]);
    expect(screen.getByText("checking the file first")).toBeInTheDocument();
    expect(screen.getAllByText("project.read_file")).toHaveLength(1);
  });

  it("streams the answer as prose, never inside a card", () => {
    // The headline defect: a tool-bearing run streamed its answer inside the
    // activity card as inert secondary-ink text, then re-rendered it as markdown
    // prose in the transcript when the run settled.
    const { container } = render(
      <ChatSurface
        activity={[
          { type: "requested", version: 1, invocation_id: "inv-1", run_id: "run-1", tool_id: "project.read_file" },
        ]}
        liveText="## Net sales"
        messages={[message]}
        runActive
      />,
    );
    // Rendered through the markdown pipeline, as a message row.
    expect(container.querySelector("h2")).not.toBeNull();
    expect(container.querySelector(".assistant-markdown")).not.toBeNull();
    // And below the tool card, not inside it.
    const rows = Array.from(container.querySelectorAll("[data-state], article")).map((node) => node.textContent ?? "");
    const card = rows.findIndex((rowText) => rowText.includes("Thinking"));
    const answer = rows.findIndex((rowText) => rowText.includes("Net sales"));
    expect(card).toBeGreaterThanOrEqual(0);
    expect(answer).toBeGreaterThan(card);
  });

  it("keeps tool-round narration inside run state but out of the visible response", () => {
    render(
      <ChatSurface
        activity={[
          { type: "requested", version: 1, invocation_id: "inv-1", run_id: "run-1", tool_id: "project.read_file" },
        ]}
        messages={[message]}
        roundProse={[{ text: "I'll check the sales table.", afterCallCount: 0 }]}
        runActive
      />,
    );
    expect(screen.queryByText("I'll check the sales table.")).not.toBeInTheDocument();
  });

  it("keeps one continuous visual owner from submit through durable settlement", () => {
    vi.stubGlobal(
      "matchMedia",
      vi.fn(() => ({ matches: true })),
    );
    const userMessage = { id: "m-user", role: "user" as const, text: "Question", sequence: BigInt(1), toolCalls: [] };
    const requested = {
      type: "requested" as const,
      version: 1 as const,
      invocation_id: "inv-1",
      run_id: "run-1",
      tool_id: "project.read_file",
    };
    const base = {
      messages: [userMessage],
      onSubmit: vi.fn(),
      streamRunId: "run-1",
    };
    const { container, rerender } = render(<ChatSurface {...base} submitting thinking />);

    expect(screen.getByText("Thinking")).toBeInTheDocument();

    // Accepted but not yet visibly productive. Hidden round narration cannot
    // take the pill's slot and create the 3-5 second blank gap.
    rerender(
      <ChatSurface
        {...base}
        roundProse={[{ text: "Hidden loop narration", afterCallCount: 0 }]}
        runActive
        streaming
        thinking
      />,
    );
    expect(screen.getByText("Thinking")).toBeInTheDocument();

    rerender(<ChatSurface {...base} activity={[requested]} runActive streaming thinking />);
    expect(screen.getAllByRole("button", { name: /Thinking/ })).toHaveLength(1);

    rerender(
      <ChatSurface
        {...base}
        activity={[requested]}
        answerStreaming
        liveText="The answer."
        runActive
        streaming
        thinking
      />,
    );
    expect(container.querySelectorAll(".assistant-markdown")).toHaveLength(1);
    expect(container.querySelector(".streaming-caret")).not.toBeNull();
    const virtualHeight = container.querySelector("ul")?.style.height;

    // Provider completion flushes once and removes only the caret. The held
    // answer stays until the durable snapshot is ready.
    rerender(<ChatSurface {...base} activity={[requested]} liveText="The answer." />);
    expect(container.querySelectorAll(".assistant-markdown")).toHaveLength(1);
    expect(container.querySelector(".streaming-caret")).toBeNull();

    rerender(
      <ChatSurface
        {...base}
        messages={[
          userMessage,
          {
            id: "m-tool",
            role: "assistant",
            text: "",
            sequence: BigInt(2),
            toolCalls: [
              {
                call_id: "inv-1",
                name: "project.read_file",
                status: "answered",
                arguments: null,
                arguments_truncated: false,
                result: null,
                result_truncated: false,
                step: null,
              },
            ],
          },
          { id: "m-answer", role: "assistant", text: "The answer.", sequence: BigInt(3), toolCalls: [] },
        ]}
      />,
    );
    expect(container.querySelectorAll(".assistant-markdown")).toHaveLength(1);
    expect(container.querySelector(".message-enter")).toBeNull();
    expect(container.querySelector("ul")?.style.height).toBe(virtualHeight);
    const currentRunRows = container.querySelectorAll("[data-current-run-index]");
    expect(currentRunRows[currentRunRows.length - 1]).toHaveTextContent("The answer.");
  });
});
