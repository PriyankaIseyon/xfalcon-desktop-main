import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ToolInvocationEvent } from "../../lib/generated/contracts";
import { DEFAULT_APPEARANCE, chooseAppearance, resetAppearanceForTests } from "../../app/appearance";
import { ChatSurface } from "./ChatSurface";
import { LiveRunTimeline } from "./LiveRunTimeline";
import { liveRunHasContent } from "./runSurface";
import type { ChatMessageRow } from "./chatStore";

// Payload disclosure is off in the shipped build and irrelevant here; this suite
// is about WHICH block a piece of content becomes and WHERE it sits.
vi.mock("./activityDisclosure", () => ({
  SHOW_TOOL_PAYLOADS: false,
  SHOW_TOOL_STEP_SUMMARY: false,
}));

async function showReasoning() {
  // Exercise the real appearance store. No native host is available in jsdom;
  // the choice is applied before the persistence leg rejects.
  await chooseAppearance({ ...DEFAULT_APPEARANCE, show_reasoning: true }, { dataset: {} }).catch(() => {});
}

afterEach(cleanup);
afterEach(resetAppearanceForTests);
afterEach(() => vi.unstubAllGlobals());

function requested(invocationId: string, toolId: string): ToolInvocationEvent {
  return {
    type: "requested",
    version: 1,
    invocation_id: invocationId,
    run_id: "run-1",
    tool_id: toolId,
  } as unknown as ToolInvocationEvent;
}

function completed(invocationId: string): ToolInvocationEvent {
  return {
    type: "completed",
    version: 1,
    invocation_id: invocationId,
    truncated: false,
  } as unknown as ToolInvocationEvent;
}

function timeline(overrides: Partial<Parameters<typeof LiveRunTimeline>[0]> = {}) {
  return <LiveRunTimeline activity={[]} reasoning={[]} reasoningTrimmed={false} runActive {...overrides} />;
}

const question: ChatMessageRow = { id: "m-1", role: "user", text: "ask", sequence: 1n, toolCalls: [] };

/** The whole surface for a run in flight: the pill AND the answer, as the reader sees them. */
function surface(overrides: Partial<Parameters<typeof ChatSurface>[0]> = {}) {
  return <ChatSurface activity={[]} messages={[question]} reasoning={[]} runActive {...overrides} />;
}

/** Whether `first` comes before `second` in the document. */
function precedes(first: Element, second: Element): boolean {
  return Boolean(first.compareDocumentPosition(second) & Node.DOCUMENT_POSITION_FOLLOWING);
}

/** The reader's reduced-motion preference, so a streamed answer shows whole. */
function reduceMotion() {
  vi.stubGlobal(
    "matchMedia",
    vi.fn(() => ({ matches: true })),
  );
}

describe("LiveRunTimeline (spec: chat-run-activity)", () => {
  it("draws nothing for a run that has produced nothing", () => {
    const { container } = render(timeline());
    expect(container).toBeEmptyDOMElement();
    expect(liveRunHasContent({ activity: [], roundProse: [], reasoning: [], liveText: "" }, true)).toBe(false);
  });

  it("does not let retained but hidden round prose claim the live visual slot", () => {
    expect(
      liveRunHasContent(
        {
          activity: [],
          roundProse: [{ text: "Hidden loop narration", afterCallCount: 0 }],
          reasoning: [],
          liveText: "",
        },
        true,
      ),
    ).toBe(false);

    render(
      <ChatSurface
        messages={[{ id: "m-user", role: "user", text: "Question", sequence: BigInt(1), toolCalls: [] }]}
        roundProse={[{ text: "Hidden loop narration", afterCallCount: 0 }]}
        runActive
        thinking
      />,
    );
    expect(screen.getByText("Thinking")).toBeInTheDocument();
    expect(screen.queryByText("Hidden loop narration")).not.toBeInTheDocument();
  });

  it("pipes thinking and every tool through one pill, then only the final answer", async () => {
    await showReasoning();
    reduceMotion();
    const { container } = render(
      surface({
        activity: [requested("inv-1", "mcp.tenant.ida_query"), completed("inv-1")],
        reasoning: [
          { text: "I need the schema.", afterCallCount: 0 },
          { text: "The schema says users.", afterCallCount: 1 },
        ],
        roundProse: [{ text: "Checking the schema.", afterCallCount: 0 }],
        liveText: "Net sales came to $75.6M.",
        answerStreaming: true,
      }),
    );
    // Answer prose has started, so the activity detail yields attention until
    // the reader explicitly asks to inspect it.
    fireEvent.click(screen.getByRole("button", { name: /Thinking/ }));
    const pill = screen.getByLabelText("Run activity");
    expect(pill.textContent).toContain("I need the schema.");
    expect(pill.textContent).toContain("The schema says users.");
    expect(pill.textContent).toContain("ida_query");
    expect(pill.textContent).not.toContain("Checking the schema.");
    expect(pill.textContent).not.toContain("Net sales");
    const answer = container.querySelector(".assistant-markdown");
    expect(answer).toHaveTextContent("Net sales came to $75.6M.");
    expect(precedes(pill, answer as Element)).toBe(true);
    expect(container.querySelectorAll(".assistant-markdown")).toHaveLength(1);
  });

  it("keeps all tool rows in one pill and suppresses tool-round narration", () => {
    const { container } = render(
      timeline({
        activity: [
          requested("inv-1", "mcp.tenant.first_tool"),
          requested("inv-2", "mcp.tenant.second_tool"),
          requested("inv-3", "mcp.tenant.third_tool"),
        ],
      }),
    );
    fireEvent.click(screen.getByRole("button", { name: /Thinking/ }));
    expect(container.children).toHaveLength(1);
    expect(container.textContent).toContain("Thinking");
    expect(container.textContent).toContain("first_tool");
    expect(container.textContent).toContain("third_tool");
  });

  it("does not draw round narration, and draws no answer of its own", () => {
    render(
      surface({
        activity: [requested("inv-1", "mcp.tenant.first_tool")],
        // Captured after one row. It is loop progress, not another response.
        roundProse: [{ text: "Now the totals.", afterCallCount: 1 }],
      }),
    );
    expect(screen.queryByText("Now the totals.")).not.toBeInTheDocument();
    expect(document.querySelector(".assistant-markdown")).toBeNull();
  });

  it("streams the answer as markdown prose, never as card text", () => {
    reduceMotion();
    const { container } = render(
      surface({
        activity: [requested("inv-1", "mcp.tenant.ida_query")],
        answerStreaming: true,
        liveText: "## Net sales\n\nAcross **132,650** orders.",
      }),
    );
    expect(container.querySelector("h2")).not.toBeNull();
    expect(container.querySelector("strong")).not.toBeNull();
    // The caret trails the row that is still arriving, and only that row.
    expect(container.querySelectorAll(".streaming-caret")).toHaveLength(1);
    const caret = container.querySelector(".streaming-text-caret");
    expect(caret).not.toBeNull();
    expect(caret?.closest(".assistant-markdown")).not.toBeNull();
    // The first answer token hands attention from the tool detail to the prose.
    expect(screen.getByRole("button", { name: /Thinking/ })).toHaveAttribute("aria-expanded", "false");
  });

  it("smooths a provider-sized burst in the real live answer row and finishes the reveal after settle", () => {
    let nextFrameId = 1;
    let clock = 0;
    const frames = new Map<number, FrameRequestCallback>();
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      const id = nextFrameId;
      nextFrameId += 1;
      frames.set(id, callback);
      return id;
    });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
    vi.stubGlobal(
      "matchMedia",
      vi.fn(() => ({ matches: false })),
    );
    const flushFrame = () => {
      clock += 1000 / 60;
      const pending = [...frames.values()];
      frames.clear();
      for (const callback of pending) callback(clock);
    };
    const answer = "A complete answer delivered in one provider burst. ".repeat(8);
    const activity = [requested("inv-1", "mcp.tenant.ida_query")];
    const { container, rerender } = render(surface({ activity, answerStreaming: true, liveText: answer }));

    expect(container.querySelector(".assistant-markdown")).toBeNull();
    act(flushFrame);
    const partial = container.querySelector(".assistant-markdown")?.textContent ?? "";
    expect(partial.length).toBeGreaterThan(0);
    expect(partial.length).toBeLessThan(answer.length);

    // The provider finishing does not cut the reveal short: the row keeps its
    // streaming caret until the whole answer is on screen.
    rerender(surface({ activity, answerStreaming: false, liveText: answer }));
    expect(container.querySelector(".assistant-markdown")?.textContent).not.toBe(answer);
    expect(container.querySelector(".streaming-caret")).not.toBeNull();
    for (let guard = 0; frames.size > 0 && guard < 300; guard += 1) act(flushFrame);
    expect(container.querySelector(".assistant-markdown")).toHaveTextContent(answer.trim());
    expect(container.querySelector(".streaming-caret")).toBeNull();
    expect(frames.size).toBe(0);
  });

  it("announces the arriving answer and nothing else", () => {
    // A live region on the transcript's scroll container made every
    // virtualization mount an announcement; the answer being written is the one
    // thing on this surface a screen reader should hear as it arrives.
    reduceMotion();
    const { container } = render(
      surface({
        roundProse: [{ text: "Checking the schema.", afterCallCount: 0 }],
        liveText: "Net sales came to $75.6M.",
        answerStreaming: true,
      }),
    );
    const live = container.querySelectorAll("[aria-live='polite']");
    const inTranscript = Array.from(live).filter((node) => node.querySelector(".assistant-markdown"));
    expect(inTranscript).toHaveLength(1);
    expect(inTranscript[0]?.textContent).toContain("Net sales came to $75.6M.");
    expect(screen.queryByText("Checking the schema.")).not.toBeInTheDocument();
  });

  it("opens the response once, on the arriving answer", () => {
    reduceMotion();
    const { container } = render(
      surface({
        activity: [requested("inv-1", "mcp.tenant.ida_query")],
        roundProse: [{ text: "Checking the schema.", afterCallCount: 0 }],
        liveText: "Net sales came to $75.6M.",
        answerStreaming: true,
      }),
    );
    const ribbons = container.querySelectorAll(".response-ribbon");
    expect(ribbons).toHaveLength(1);
    expect(ribbons[0]?.closest("article")?.textContent).toContain("Net sales came to $75.6M.");
  });

  it("draws no ribbon when the transcript already opened the response", () => {
    reduceMotion();
    const opened: ChatMessageRow = {
      id: "m-2",
      role: "assistant",
      text: "An earlier part of this response.",
      sequence: 2n,
      toolCalls: [],
    };
    const { container } = render(
      surface({ messages: [question, opened], liveText: "Net sales came to $75.6M.", answerStreaming: true }),
    );
    expect(container.querySelectorAll(".response-ribbon")).toHaveLength(0);
  });

  it("withholds reasoning when the preference is off, and nothing else", async () => {
    const target = { dataset: {} as Record<string, string> };
    await chooseAppearance({ ...DEFAULT_APPEARANCE, show_reasoning: false }, target).catch(() => {});
    reduceMotion();
    const { container } = render(
      surface({
        activity: [requested("inv-1", "mcp.tenant.ida_query")],
        reasoning: [{ text: "I need the schema.", afterCallCount: 0 }],
        liveText: "Net sales came to $75.6M.",
        answerStreaming: true,
      }),
    );
    expect(screen.getByRole("button", { name: /Thinking/ })).toBeInTheDocument();
    expect(container.querySelector(".assistant-markdown")).toHaveTextContent("Net sales came to $75.6M.");
    expect(screen.queryByText("I need the schema.")).toBeNull();
  });
});

/**
 * The settle, asserted as an equivalence.
 *
 * This is the whole design in one test: the block sequence the live timeline
 * draws from live state must be the block sequence the transcript draws from the
 * durable messages that replace it. If they differ, something on screen moves,
 * changes typography, or disappears at the moment the run settles — which is the
 * defect class this change exists to remove (spec: chat-run-activity, "The settle
 * changes nothing about the answer").
 *
 * Rendered through `ChatSurface` because it owns both halves, and comparing the
 * transcript column's text block by block.
 */
describe("the settle (spec: chat-run-activity)", () => {
  const question: ChatMessageRow = {
    id: "m-1",
    role: "user",
    text: "what were net sales?",
    sequence: 1n,
    toolCalls: [],
  };
  const call = {
    version: 1,
    call_id: "call-1",
    name: "ida_query",
    status: "answered",
    arguments: null,
    result: null,
    arguments_truncated: false,
    result_truncated: false,
    step: null,
  };

  function textOf(container: HTMLElement): string {
    return (container.querySelector("ul.list-none")?.parentElement?.textContent ?? "")
      .replace(/Copy message/g, "")
      .replace(/\s+/g, " ")
      .trim();
  }

  it("renders the same sequence before and after the durable snapshot lands", () => {
    const live = render(
      <ChatSurface
        activity={[requested("inv-1", "mcp.tenant.ida_query"), completed("inv-1")]}
        liveText="Net sales came to $75.6M."
        messages={[question]}
        roundProse={[{ text: "I'll check the sales table.", afterCallCount: 0 }]}
        runActive={false}
      />,
    );
    const before = textOf(live.container);
    cleanup();

    const settled = render(
      <ChatSurface
        activity={[]}
        messages={[
          question,
          {
            id: "m-2",
            role: "assistant",
            text: "I'll check the sales table.",
            sequence: 2n,
            toolCalls: [call],
          } as ChatMessageRow,
          { id: "m-3", role: "assistant", text: "Net sales came to $75.6M.", sequence: 3n, toolCalls: [] },
        ]}
      />,
    );
    const after = textOf(settled.container);

    expect(before).toContain("what were net sales?");
    expect(before).not.toContain("I'll check the sales table.");
    expect(before).toContain("Net sales came to $75.6M.");
    expect(after).toContain("what were net sales?");
    expect(after).not.toContain("I'll check the sales table.");
    expect(after).toContain("Net sales came to $75.6M.");
    expect(after).toContain("1 tool used");
  });

  it("keeps the answer out of any card in both states", () => {
    const live = render(
      <ChatSurface
        activity={[requested("inv-1", "mcp.tenant.ida_query")]}
        liveText="Net sales came to $75.6M."
        messages={[question]}
        runActive
      />,
    );
    // The answer is inside a prose row…
    const articles = Array.from(live.container.querySelectorAll("article"));
    expect(articles.some((article) => article.textContent?.includes("Net sales came to $75.6M."))).toBe(true);
    // …and reopening the auto-collapsed card shows that it holds tool rows only.
    fireEvent.click(screen.getByRole("button", { name: /Thinking/ }));
    expect(screen.getByLabelText("Run activity").textContent).not.toContain("Net sales");
  });
});

/**
 * The ribbon rule is expressed twice — as a walk inside `buildDisplayItems`, and
 * as `responseAlreadyOpen` for the live timeline, which builds from live state
 * and cannot see the walk. This holds the two together: whatever the mix of
 * durable turns, one response gets exactly one boundary (spec:
 * chat-thread-layout).
 */
describe("one response, one boundary (spec: chat-thread-layout)", () => {
  const user: ChatMessageRow = { id: "m-1", role: "user", text: "ask", sequence: 1n, toolCalls: [] };
  const toolCall = {
    version: 1,
    call_id: "call-1",
    name: "ida_query",
    status: "answered",
    arguments: null,
    result: null,
    arguments_truncated: false,
    result_truncated: false,
    step: null,
  };
  const round = (id: string, text: string): ChatMessageRow =>
    ({ id, role: "assistant", text, sequence: 2n, toolCalls: [toolCall] }) as ChatMessageRow;

  const cases: Array<[name: string, messages: ChatMessageRow[]]> = [
    ["nothing durable yet", [user]],
    ["a narrated round is durable", [user, round("m-2", "I'll check.")]],
    ["a silent round is durable", [user, round("m-2", "")]],
    ["two narrated rounds are durable", [user, round("m-2", "First."), round("m-3", "Second.")]],
    ["a silent round then a narrated one", [user, round("m-2", ""), round("m-3", "Second.")]],
  ];

  for (const [name, messages] of cases) {
    it(`draws one ribbon when ${name}`, () => {
      const { container } = render(<ChatSurface activity={[]} liveText="The answer." messages={messages} runActive />);
      expect(container.querySelectorAll(".response-ribbon")).toHaveLength(1);
    });
  }
});
