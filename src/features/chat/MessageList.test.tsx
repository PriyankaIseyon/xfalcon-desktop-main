import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_APPEARANCE, chooseAppearance, resetAppearanceForTests } from "../../app/appearance";
import { MessageList } from "./MessageList";
import { REMEASURE_EVENT } from "./ToolActivityCard";
import type { ChatMessageRow } from "./chatStore";

// The shipped default withholds payloads and step summaries
// (`./activityDisclosure`). The code behind them is intact and one flag away, so
// this suite forces disclosure ON and keeps proving the properties that matter
// if it is ever turned back on — hostile content stays inert, paths stay
// literal, nested panels still settle their row. What the DEFAULT draws is
// pinned separately, in `activityDisclosure.test.tsx`. Reasoning display is not
// a flag any more but a preference, explicitly enabled for these cases.
vi.mock("./activityDisclosure", () => ({
  SHOW_TOOL_PAYLOADS: true,
  SHOW_TOOL_STEP_SUMMARY: true,
}));

async function showReasoning() {
  // Exercise the real appearance store. No native host is available in jsdom;
  // the choice is applied before the persistence leg rejects.
  await chooseAppearance({ ...DEFAULT_APPEARANCE, show_reasoning: true }, { dataset: {} }).catch(() => {});
}

beforeEach(showReasoning);
afterEach(cleanup);
afterEach(resetAppearanceForTests);

beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    const isScrollContainer = this instanceof HTMLDivElement && this.className.includes("overflow-y-auto");
    const height = isScrollContainer ? 480 : 72;
    return {
      width: 640,
      height,
      top: 0,
      left: 0,
      bottom: height,
      right: 640,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    };
  });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get: () => 480,
  });
});

afterEach(() => {
  vi.restoreAllMocks();
});

function row(index: number): ChatMessageRow {
  return {
    id: `m-${index}`,
    role: index % 2 === 0 ? "user" : "assistant",
    text: `Message ${index}`,
    sequence: BigInt(index),
    toolCalls: [],
  };
}

function toolCall(index: number) {
  return {
    call_id: `call-${index}`,
    name: `mcp_tool_${index}`,
    status: "answered" as const,
    arguments: null,
    arguments_truncated: false,
    result: null,
    result_truncated: false,
    step: null,
  };
}

function toolTurn(index: number, calls: number, text = ""): ChatMessageRow {
  return {
    id: `m-${index}`,
    role: "assistant",
    text,
    sequence: BigInt(index),
    toolCalls: Array.from({ length: calls }, (_, call) => toolCall(index * 10 + call)),
  };
}

describe("MessageList", () => {
  it("renders message content for a typical transcript", () => {
    const messages = [row(0), row(1)];
    render(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={messages} />
      </div>,
    );
    expect(screen.getByText("Message 0")).toBeInTheDocument();
    expect(screen.getByText("Message 1")).toBeInTheDocument();
  });

  it("keeps mounted rows bounded for very large transcripts", async () => {
    const messages = Array.from({ length: 10_000 }, (_, index) => row(index));
    const { container } = render(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={messages} />
      </div>,
    );
    await waitFor(() => {
      expect(container.querySelectorAll("article").length).toBeGreaterThan(0);
    });
    const mounted = container.querySelectorAll("li").length;
    expect(mounted).toBeLessThan(64);
  });

  it("animates only appended rows — never the initial bulk load", () => {
    const bulk = [row(0), row(1)];
    const { container, rerender } = render(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={bulk} />
      </div>,
    );
    expect(container.querySelector(".message-enter")).toBeNull();

    rerender(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={[...bulk, row(2)]} />
      </div>,
    );
    const entering = container.querySelectorAll(".message-enter");
    expect(entering.length).toBe(1);
    expect(entering[0]?.textContent).toContain("Message 2");
  });

  it("does not animate a wholesale conversation swap", () => {
    const first = [row(0), row(1)];
    const { container, rerender } = render(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={first} />
      </div>,
    );
    rerender(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={[row(10), row(11), row(12)]} />
      </div>,
    );
    expect(container.querySelector(".message-enter")).toBeNull();
  });

  it("does not replay entrance motion when the live answer becomes its durable row", () => {
    const first = [row(0)];
    const { container, rerender } = render(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={first} tail={<div data-testid="live-answer">The answer.</div>} tailHasLatestAnswer />
      </div>,
    );
    const virtualHeightBefore = container.querySelector("ul")?.style.height;

    rerender(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={[...first, { ...row(1), text: "The answer." }]} />
      </div>,
    );

    expect(screen.queryByTestId("live-answer")).not.toBeInTheDocument();
    expect(screen.getByText("The answer.")).toBeInTheDocument();
    expect(container.querySelector(".message-enter")).toBeNull();
    expect(container.querySelector("ul")?.style.height).toBe(virtualHeightBefore);
    expect(container.querySelector("[data-current-run-index='0']")).toHaveTextContent("The answer.");
  });

  it("folds a whole tool sequence into ONE umbrella above the final answer", () => {
    const messages: ChatMessageRow[] = [
      { ...row(0) }, // user question
      toolTurn(1, 2), // tool round, no prose
      toolTurn(2, 1), // second round, no prose
      { ...row(3), role: "assistant", text: "The answer." }, // final answer
    ];
    render(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={messages} />
      </div>,
    );
    expect(screen.getByText("3 tools used")).toBeInTheDocument();
    expect(screen.queryByText("2 tools used")).not.toBeInTheDocument();
    expect(screen.queryByText("1 tool used")).not.toBeInTheDocument();
    expect(screen.getByText("The answer.")).toBeInTheDocument();
  });

  it("withholds only the current run's durable activity while its live pill owns it", () => {
    const messages: ChatMessageRow[] = [
      row(0),
      toolTurn(1, 1),
      { ...row(2), role: "assistant", text: "A projected answer." },
    ];
    render(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={messages} suppressCurrentRunActivity />
      </div>,
    );

    expect(screen.queryByText("1 tool used")).not.toBeInTheDocument();
    expect(screen.getByText("A projected answer.")).toBeInTheDocument();
  });

  it("keeps tool-round narration out of the transcript and renders one tool section before the answer", () => {
    const messages: ChatMessageRow[] = [
      { ...row(0) }, // user question
      toolTurn(1, 1, "I'll check what data sources are available."), // round 1: prose + call
      toolTurn(2, 4), // round 2: four calls, no prose
      { ...row(3), role: "assistant", text: "## Summary of the workspace" }, // final answer
    ];
    const { container } = render(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={messages} />
      </div>,
    );

    const rows = Array.from(container.querySelectorAll("[data-index], [data-current-run-index]")).map(
      (row) => row.textContent ?? "",
    );
    const prose = rows.findIndex((text) => text.includes("I'll check what data sources"));
    const umbrella = rows.findIndex((text) => text.includes("5 tools used"));
    const answer = rows.findIndex((text) => text.includes("Summary of the workspace"));
    expect(prose).toBe(-1);
    expect(umbrella).toBeGreaterThanOrEqual(0);
    expect(answer).toBeGreaterThan(umbrella);
    expect(screen.queryByText("1 tool used")).not.toBeInTheDocument();
    expect(screen.queryByText("4 tools used")).not.toBeInTheDocument();
  });

  it("does not split the response tool section when narration lands between rounds", () => {
    const messages: ChatMessageRow[] = [
      { ...row(0) },
      toolTurn(1, 1), // round 1: one call, no prose
      toolTurn(2, 4, "Now I need the totals."), // round 2: prose, then four calls
      { ...row(3), role: "assistant", text: "Done." },
    ];
    render(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={messages} />
      </div>,
    );
    expect(screen.getByText("5 tools used")).toBeInTheDocument();
    expect(screen.queryByText("1 tool used")).not.toBeInTheDocument();
    expect(screen.queryByText("4 tools used")).not.toBeInTheDocument();
    expect(screen.queryByText("Now I need the totals.")).not.toBeInTheDocument();
  });

  it("opens one response ribbon on the final answer", () => {
    const messages: ChatMessageRow[] = [
      { ...row(0) },
      toolTurn(1, 1, "I'll check the sales table."),
      { ...row(2), role: "assistant", text: "Net sales came to $75.6M." },
    ];
    const { container } = render(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={messages} />
      </div>,
    );
    expect(container.querySelectorAll(".response-ribbon")).toHaveLength(1);
    const ribboned = container.querySelector(".response-ribbon")?.closest("article");
    expect(ribboned?.textContent).toContain("Net sales came to $75.6M.");
    expect(screen.queryByText("I'll check the sales table.")).not.toBeInTheDocument();
  });

  // The handover: reasoning is never persisted, so when the live timeline retires
  // the transcript is the only place left that can hold it — in the same
  // positions the timeline drew it (spec: chat-run-activity).
  it("carries the settled run's reasoning as unified disclosures, in order", () => {
    const messages: ChatMessageRow[] = [
      row(0), // user question
      toolTurn(1, 1, "Checking the schema."),
      { ...row(2), role: "assistant", text: "The answer." },
    ];
    const { container } = render(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList
          messages={messages}
          reasoning={[
            { text: "I need the schema.", afterCallCount: 0 },
            { text: "The schema says users.", afterCallCount: 1 },
          ]}
        />
      </div>,
    );
    const disclosures = screen.getAllByRole("button", { name: /Thought process/ });
    expect(disclosures).toHaveLength(1);

    // One unified activity card reads above the one visible assistant answer.
    const rows = Array.from(container.querySelectorAll("[data-index], [data-current-run-index]")).map(
      (row) => row.textContent ?? "",
    );
    const prose = rows.findIndex((text) => text.includes("Checking the schema."));
    const unifiedActivity = rows.findIndex((text) => text.includes("Thought process · 1 tool used"));
    const answer = rows.findIndex((text) => text.includes("The answer."));
    expect(prose).toBe(-1);
    expect(unifiedActivity).toBeGreaterThanOrEqual(0);
    expect(answer).toBeGreaterThan(unifiedActivity);

    fireEvent.click(disclosures[0]);
    expect(screen.getByText(/I need the schema/)).toBeInTheDocument();
    expect(screen.getByText(/The schema says users/)).toBeInTheDocument();
    expect(screen.getByText("mcp_tool_10")).toBeInTheDocument();
  });

  it("expands unified activity disclosure onto both reasoning and tool rows", () => {
    const messages: ChatMessageRow[] = [
      row(0),
      toolTurn(1, 1, "Checking the schema."),
      { ...row(2), role: "assistant", text: "The answer." },
    ];
    render(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={messages} reasoning={[{ text: "I need the schema.", afterCallCount: 0 }]} />
      </div>,
    );
    fireEvent.click(screen.getByRole("button", { name: /Thought process · 1 tool used/ }));
    expect(screen.getByText("I need the schema.")).toBeInTheDocument();
    expect(screen.getByText("mcp_tool_10")).toBeInTheDocument();
  });

  it("keeps the newest run's reasoning off an earlier run's turns", () => {
    const messages: ChatMessageRow[] = [
      row(0), // first question
      toolTurn(1, 1),
      { ...row(2), role: "assistant", text: "First answer." },
      { ...row(4), role: "user", text: "Second question." },
      toolTurn(5, 1),
      { ...row(6), role: "assistant", text: "Second answer." },
    ];
    const { container } = render(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={messages} reasoning={[{ text: "only the newest", afterCallCount: 0 }]} />
      </div>,
    );
    const disclosures = screen.getAllByRole("button", { name: /Thought process/ });
    // Exactly one, and it belongs to the newest run.
    expect(disclosures).toHaveLength(1);
    fireEvent.click(disclosures[0]);
    expect(screen.getByText("only the newest")).toBeInTheDocument();

    const rows = Array.from(container.querySelectorAll("[data-index], [data-current-run-index]")).map(
      (row) => row.textContent ?? "",
    );
    expect(rows.findIndex((text) => text.includes("only the newest"))).toBeGreaterThan(
      rows.findIndex((text) => text.includes("First answer.")),
    );
  });

  it("gives a run that reasoned and called nothing a disclosure of its own", () => {
    const messages: ChatMessageRow[] = [row(0), { ...row(2), role: "assistant", text: "The answer." }];
    render(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={messages} reasoning={[{ text: "no tools needed", afterCallCount: 0 }]} />
      </div>,
    );
    // Named for what it holds — a tool umbrella labelled "0 tools used" would
    // name the one thing that did not happen.
    fireEvent.click(screen.getByRole("button", { name: /Thought process/ }));
    expect(screen.getByText("no tools needed")).toBeInTheDocument();
    expect(screen.queryByText(/tools? used/)).not.toBeInTheDocument();
  });

  it("draws no row for a tool-result turn, and does not let one split a run", () => {
    // The projection carries tool-result turns (`wire.rs`), and their single
    // content part is a reference rather than text, so their projected text is
    // empty. As a display item each one measured a virtualized row, drew a stray
    // copy button into it, and closed the umbrella between one round's calls and
    // the next round's.
    const messages: ChatMessageRow[] = [
      { ...row(0) },
      toolTurn(1, 2),
      { id: "m-tool-1", role: "system", text: "", sequence: BigInt(2), toolCalls: [] },
      { id: "m-tool-2", role: "system", text: "", sequence: BigInt(3), toolCalls: [] },
      toolTurn(4, 1),
      { ...row(5), role: "assistant", text: "The answer." },
    ];
    const { container } = render(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={messages} />
      </div>,
    );
    // One umbrella over all three calls, and no empty rows between them.
    expect(screen.getByText("3 tools used")).toBeInTheDocument();
    const rows = Array.from(container.querySelectorAll("[data-index], [data-current-run-index]"));
    expect(rows.map((node) => node.textContent?.trim() === "")).not.toContain(true);
    // One copy button per turn that has something to copy: the question and the
    // answer.
    expect(screen.getAllByRole("button", { name: /Copy message/ })).toHaveLength(2);
  });

  it("closes the umbrella at a user turn — separate sequences stay separate", () => {
    const messages: ChatMessageRow[] = [
      row(0),
      toolTurn(1, 1),
      { ...row(2) }, // user again
      toolTurn(3, 2),
      { ...row(4), role: "assistant", text: "Done." },
    ];
    const { container } = render(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={messages} />
      </div>,
    );
    expect(screen.getByText("1 tool used")).toBeInTheDocument();
    expect(screen.getByText("2 tools used")).toBeInTheDocument();
    expect(container.querySelectorAll("article").length).toBeGreaterThan(0);
  });

  it("renders nothing at all for an empty assistant turn", () => {
    const empty: ChatMessageRow = { id: "m-empty", role: "assistant", text: "  ", sequence: BigInt(2), toolCalls: [] };
    const { container } = render(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={[row(0), empty, { ...row(3), role: "assistant", text: "Real." }]} />
      </div>,
    );
    const articles = [...container.querySelectorAll("article")];
    expect(articles).toHaveLength(2); // the user turn and the real answer
  });

  it("hands the accent rail to the last assistant turn only", () => {
    const messages = [row(0), row(1), row(2), row(3)]; // assistants at 1 and 3
    const { container } = render(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={messages} />
      </div>,
    );
    const latest = container.querySelectorAll("article.assistant-accent-latest");
    expect(latest.length).toBe(1);
    expect(latest[0]?.textContent).toContain("Message 3");
    const history = container.querySelectorAll("article.assistant-accent");
    expect(history.length).toBe(1);
    expect(history[0]?.textContent).toContain("Message 1");
  });

  it("renders the tail inside the scrolling reading column, after the transcript", () => {
    const { container } = render(
      <div className="flex h-[480px] w-[640px] flex-col">
        <MessageList messages={[row(0)]} tail={<div data-testid="live-tail">Working</div>} />
      </div>,
    );
    const column = container.querySelector(".chat-column-messages");
    const tail = column?.querySelector("[data-testid='live-tail']");
    expect(tail).toBeTruthy();
    // The tail follows the virtualized list — where the next turn will land.
    const list = column?.querySelector("ul");
    expect(list && tail ? list.compareDocumentPosition(tail) & Node.DOCUMENT_POSITION_FOLLOWING : 0).toBeTruthy();
  });

  /**
   * The listener half of the settle contract (spec: chat-motion-system). The
   * reporter's side — that a toggle reports at all with motion off — is tested
   * in `ToolActivityCard.test.tsx`; these cases feed the report in by hand and
   * pin what the list does with it, which is the part that closes the gap.
   */
  describe("settle reports", () => {
    /** Overrides the blanket 72 for one row, as an open disclosure would. */
    function measureAs(element: HTMLElement, height: number) {
      vi.spyOn(element, "getBoundingClientRect").mockReturnValue({
        width: 640,
        height,
        top: 0,
        left: 0,
        bottom: height,
        right: 640,
        x: 0,
        y: 0,
        toJSON: () => ({}),
      } as DOMRect);
    }

    /** A report as the trail dispatches one: payload-free and bubbling. */
    const reportSettle = (from: HTMLElement) => fireEvent(from, new CustomEvent(REMEASURE_EVENT, { bubbles: true }));

    /** Total size is where the item-size cache becomes observable. */
    const totalPx = (container: HTMLElement) => Number.parseFloat(container.querySelector("ul")?.style.height ?? "NaN");

    function renderRows() {
      const { container } = render(
        <div className="flex h-[480px] w-[640px] flex-col">
          <MessageList messages={[row(0), row(1), row(2), row(3), row(4), row(5)]} />
        </div>,
      );
      const target = container.querySelector<HTMLElement>('[data-index="1"]');
      expect(target).not.toBeNull();
      return { container, target: target as HTMLElement, collapsedPx: totalPx(container) };
    }

    it("remeasures a row after a disclosure grows and shrinks", async () => {
      const { container, target, collapsedPx } = renderRows();

      measureAs(target, 200); // the disclosure opens
      reportSettle(target);
      await waitFor(() => expect(totalPx(container)).toBeGreaterThan(collapsedPx));

      measureAs(target, 72); // …and closes again
      reportSettle(target);
      // The final measurement must converge on the collapsed height.
      await waitFor(() => expect(totalPx(container)).toBe(collapsedPx));
    });

    it("converges when one toggle is reported twice", async () => {
      const { container, target, collapsedPx } = renderRows();
      measureAs(target, 200);
      reportSettle(target);
      await waitFor(() => expect(totalPx(container)).toBeGreaterThan(collapsedPx));

      measureAs(target, 72);
      reportSettle(target);
      reportSettle(target);
      // The report carries no height, so the second read agrees with the first.
      await waitFor(() => expect(totalPx(container)).toBe(collapsedPx));
    });

    it("ignores a report from outside any row", () => {
      const { container, collapsedPx } = renderRows();
      const column = container.querySelector<HTMLElement>(".chat-column-messages");
      expect(column).not.toBeNull();

      reportSettle(column as HTMLElement);

      expect(totalPx(container)).toBe(collapsedPx);
    });

    it("accepts a row shrinking without a width change or disclosure event", async () => {
      const observers: Array<{ callback: ResizeObserverCallback; targets: Set<Element> }> = [];
      vi.stubGlobal(
        "ResizeObserver",
        class {
          private readonly entry: (typeof observers)[number];
          constructor(callback: ResizeObserverCallback) {
            this.entry = { callback, targets: new Set() };
            observers.push(this.entry);
          }
          observe(target: Element) {
            this.entry.targets.add(target);
          }
          unobserve(target: Element) {
            this.entry.targets.delete(target);
          }
          disconnect() {
            this.entry.targets.clear();
          }
        },
      );
      try {
        const { container, target, collapsedPx } = renderRows();
        const resizeRow = (height: number) => {
          measureAs(target, height);
          const entry = {
            target,
            borderBoxSize: [{ inlineSize: 640, blockSize: height }],
          } as unknown as ResizeObserverEntry;
          for (const observer of observers) {
            if (observer.targets.has(target)) observer.callback([entry], {} as ResizeObserver);
          }
        };

        // A reading-size preference or font loading can change row height
        // while the viewport width and disclosure state stay unchanged.
        resizeRow(200);
        await waitFor(() => expect(totalPx(container)).toBeGreaterThan(collapsedPx));
        resizeRow(72);
        await waitFor(() => expect(totalPx(container)).toBe(collapsedPx));
      } finally {
        cleanup();
        vi.unstubAllGlobals();
      }
    });

    /**
     * The resize half of the same rule. A step's chips wrap, so its height
     * depends on the available width — and jsdom ships no `ResizeObserver`, so
     * the observation has to be staged rather than provoked.
     */
    describe("a width change", () => {
      afterEach(() => vi.unstubAllGlobals());

      /** Records what each observer watches, so a staged resize reaches only
       * the observers of that element — the virtualizer's own per-row observers
       * are watching different nodes and must not be handed this entry. */
      function stageResizeObserver() {
        const observers: Array<{ callback: ResizeObserverCallback; targets: Element[] }> = [];
        vi.stubGlobal(
          "ResizeObserver",
          class {
            private readonly entry: { callback: ResizeObserverCallback; targets: Element[] };
            constructor(callback: ResizeObserverCallback) {
              this.entry = { callback, targets: [] };
              observers.push(this.entry);
            }
            observe(target: Element) {
              this.entry.targets.push(target);
            }
            unobserve() {}
            disconnect() {
              this.entry.targets.length = 0;
            }
          },
        );
        return (container: HTMLElement, width: number) => {
          const scroller = container.querySelector<HTMLElement>(".overflow-y-auto");
          expect(scroller).not.toBeNull();
          const entries = [{ target: scroller, contentRect: { width } }] as unknown as ResizeObserverEntry[];
          for (const observer of observers) {
            if (observer.targets.includes(scroller as HTMLElement)) {
              observer.callback(entries, {} as ResizeObserver);
            }
          }
        };
      }

      it("applies the shrink that follows a widened column", async () => {
        const resizeTo = stageResizeObserver();
        const { container, target, collapsedPx } = renderRows();
        resizeTo(container, 640); // the baseline observation, which changes nothing

        measureAs(target, 200); // chips wrapped onto two more lines
        reportSettle(target);
        await waitFor(() => expect(totalPx(container)).toBeGreaterThan(collapsedPx));

        measureAs(target, 72); // …and unwrapped when the column got wider
        resizeTo(container, 900);

        // The mounted row must return to its unwrapped height.
        await waitFor(() => expect(totalPx(container)).toBe(collapsedPx));
      });

      it("does nothing when the width did not change", async () => {
        const resizeTo = stageResizeObserver();
        const { container, target, collapsedPx } = renderRows();
        resizeTo(container, 640);

        measureAs(target, 200);
        reportSettle(target);
        await waitFor(() => expect(totalPx(container)).toBeGreaterThan(collapsedPx));
        const openPx = totalPx(container);

        // A streamed token can grow the list; unchanged viewport width must
        // not trigger a sweep of all historical rows.
        measureAs(target, 72);
        resizeTo(container, 640);

        expect(totalPx(container)).toBe(openPx);
      });
    });
  });

  describe("scroll to bottom", () => {
    /** Scrolls the virtualized container away from the bottom and lets the
     * list's own scroll listener observe it. */
    async function scrollAway(container: HTMLElement) {
      const scroller = container.querySelector<HTMLElement>(".overflow-y-auto");
      expect(scroller).toBeTruthy();
      Object.defineProperty(scroller!, "scrollHeight", { configurable: true, get: () => 4000 });
      // Establish the real starting position before moving upward. jsdom does
      // not lay out scroll height or clamp scrollTop like a browser does.
      scroller!.scrollTop = 4000;
      fireEvent.scroll(scroller!);
      scroller!.scrollTop = 0;
      fireEvent.scroll(scroller!);
      return scroller!;
    }

    it("offers no way back while the reader is already at the bottom", () => {
      render(
        <div className="flex h-[480px] w-[640px] flex-col">
          <MessageList messages={[row(0), row(1)]} />
        </div>,
      );

      expect(screen.queryByRole("button", { name: /scroll to/i })).not.toBeInTheDocument();
    });

    it("does not detach on a delayed forward scroll event after content grows", () => {
      const { container } = render(
        <div className="flex h-[480px] w-[640px] flex-col">
          <MessageList followStream messages={[row(0), row(1)]} />
        </div>,
      );
      const scroller = container.querySelector<HTMLElement>(".overflow-y-auto")!;
      let height = 4000;
      Object.defineProperty(scroller, "scrollHeight", { configurable: true, get: () => height });
      scroller.scrollTop = height;
      fireEvent.scroll(scroller);
      // The queued scroll event sees a newer layout, but no upward movement.
      height = 5000;
      fireEvent.scroll(scroller);
      expect(screen.queryByRole("button", { name: /scroll to/i })).not.toBeInTheDocument();
    });

    it("offers the way back whenever the reader has scrolled up, streaming or not", async () => {
      // The old affordance appeared only mid-stream, so a reader who scrolled
      // up in a settled conversation had to drag all the way back by hand.
      const { container } = render(
        <div className="flex h-[480px] w-[640px] flex-col">
          <MessageList messages={[row(0), row(1), row(2)]} />
        </div>,
      );

      await scrollAway(container);

      await waitFor(() => {
        expect(screen.getByRole("button", { name: "Scroll to latest message" })).toBeInTheDocument();
      });
      // Icon-only: nothing arrived while they were up there, so there is
      // nothing to announce.
      expect(screen.queryByText("New response")).not.toBeInTheDocument();
    });

    it("returns to the bottom with a short native smooth scroll", async () => {
      const user = userEvent.setup();
      const { container } = render(
        <div className="flex h-[480px] w-[640px] flex-col">
          <MessageList messages={[row(0), row(1), row(2)]} />
        </div>,
      );
      const scroller = await scrollAway(container);
      const scrollTo = vi.fn();
      Object.defineProperty(scroller, "scrollTo", { configurable: true, value: scrollTo });

      await user.click(await screen.findByRole("button", { name: "Scroll to latest message" }));

      expect(scrollTo).toHaveBeenCalledWith({ behavior: "smooth", top: 4000 });
    });

    it("says what arrived when something streamed in while the reader was away", async () => {
      const { container } = render(
        <div className="flex h-[480px] w-[640px] flex-col">
          <MessageList followStream messages={[row(0), row(1), row(2)]} />
        </div>,
      );

      await scrollAway(container);

      await waitFor(() => {
        expect(screen.getByRole("button", { name: "Scroll to the new response" })).toBeInTheDocument();
      });
      expect(screen.getByText("New response")).toBeInTheDocument();
    });

    it("has nothing to scroll to in an empty transcript", async () => {
      const { container } = render(
        <div className="flex h-[480px] w-[640px] flex-col">
          <MessageList followStream messages={[]} />
        </div>,
      );

      await scrollAway(container);

      expect(screen.queryByRole("button", { name: /scroll to/i })).not.toBeInTheDocument();
    });

    it("opens every conversation pinned to its own bottom, even one scrolled away from a moment ago", async () => {
      // `MessageList` is one persistent instance across every conversation
      // switch, not remounted per conversation -- `pinnedToBottom` is scroll
      // state that otherwise carries over unchanged from whichever
      // conversation set it last: scroll up reading history in one chat,
      // switch to another, and the switch used to land wherever that stale
      // flag left it instead of the new conversation's own latest message.
      const { container, rerender } = render(
        <div className="flex h-[480px] w-[640px] flex-col">
          <MessageList conversationId="conv-a" messages={[row(0), row(1), row(2)]} />
        </div>,
      );
      await scrollAway(container);
      await waitFor(() => {
        expect(screen.getByRole("button", { name: "Scroll to latest message" })).toBeInTheDocument();
      });

      rerender(
        <div className="flex h-[480px] w-[640px] flex-col">
          <MessageList conversationId="conv-b" messages={[row(10), row(11)]} />
        </div>,
      );

      expect(screen.queryByRole("button", { name: /scroll to/i })).not.toBeInTheDocument();
    });
  });

  describe("turn revision & regeneration", () => {
    it("renders regenerate button only on the final prose message of an assistant response", () => {
      const onRegenerate = vi.fn();
      const messages: ChatMessageRow[] = [
        {
          id: "u-1",
          role: "user",
          text: "User prompt 1",
          sequence: BigInt(1),
          toolCalls: [],
        },
        {
          id: "a-1",
          role: "assistant",
          text: "First narration segment",
          sequence: BigInt(2),
          toolCalls: [toolCall(1)],
        },
        {
          id: "a-2",
          role: "assistant",
          text: "Final concluding answer",
          sequence: BigInt(3),
          toolCalls: [],
        },
      ];

      render(
        <div className="flex h-[480px] w-[640px] flex-col">
          <MessageList messages={messages} onRegenerate={onRegenerate} />
        </div>,
      );

      const regenButtons = screen.getAllByRole("button", { name: "Regenerate response" });
      expect(regenButtons).toHaveLength(1);
    });

    it("dispatches bubbled REMEASURE_EVENT to scrollElement when entering and exiting turn edit mode", async () => {
      const user = userEvent.setup();
      const onEdit = vi.fn();
      const messages: ChatMessageRow[] = [
        {
          id: "u-edit-1",
          role: "user",
          text: "Question to edit for remeasure",
          sequence: BigInt(1),
          toolCalls: [],
        },
      ];

      const { container } = render(
        <div className="flex h-[480px] w-[640px] flex-col">
          <MessageList messages={messages} onEdit={onEdit} />
        </div>,
      );

      const scrollElement = container.querySelector(".overflow-y-auto");
      expect(scrollElement).not.toBeNull();

      const remeasureSpy = vi.fn();
      scrollElement!.addEventListener(REMEASURE_EVENT, remeasureSpy);

      await user.click(screen.getByRole("button", { name: "Edit message" }));
      await waitFor(() => {
        expect(remeasureSpy).toHaveBeenCalled();
      });

      const callCountBeforeCancel = remeasureSpy.mock.calls.length;
      await user.click(screen.getByRole("button", { name: "Cancel" }));
      await waitFor(() => {
        expect(remeasureSpy.mock.calls.length).toBeGreaterThan(callCountBeforeCancel);
      });

      scrollElement!.removeEventListener(REMEASURE_EVENT, remeasureSpy);
    });
  });
});
