import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type { ChatToolCallView, ToolInvocationEvent } from "../../lib/generated/contracts";
import { DurableToolTrail, LiveRunActivity, REMEASURE_EVENT } from "./ToolActivityCard";
import { resetAppearanceForTests } from "../../app/appearance";

// The shipped default withholds payloads and step summaries
// (`./activityDisclosure`). The code behind them is intact and one flag away, so
// this suite forces disclosure ON and keeps proving the properties that matter
// if it is ever turned back on — hostile content stays inert, paths stay
// literal, nested panels still settle their row. What the DEFAULT draws is
// pinned separately, in `activityDisclosure.test.tsx`.
vi.mock("./activityDisclosure", () => ({
  SHOW_TOOL_PAYLOADS: true,
  SHOW_TOOL_STEP_SUMMARY: true,
}));

afterEach(cleanup);
afterEach(resetAppearanceForTests);
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

const durable = (
  index: number,
  status: ChatToolCallView["status"] = "answered",
  payloads: Partial<
    Pick<ChatToolCallView, "arguments" | "arguments_truncated" | "result" | "result_truncated" | "step">
  > = {},
): ChatToolCallView => ({
  call_id: `call-${index}`,
  name: `mcp_tenant_ida_tool_${index}`,
  status,
  arguments: null,
  arguments_truncated: false,
  result: null,
  result_truncated: false,
  step: null,
  ...payloads,
});

/** A projected step summary, as `tool_step_summary` composes one. */
const summary = (
  label: string,
  paths: Array<[path: string, sensitive: boolean]> = [],
  pathTotal = paths.length,
): NonNullable<ChatToolCallView["step"]> => ({
  version: 1,
  label,
  paths: paths.map(([path, is_sensitive]) => ({ path, is_sensitive })),
  path_total: pathTotal,
});

const trail = (...calls: ChatToolCallView[]) => <DurableToolTrail calls={calls} />;

function invocationEvent(
  type: ToolInvocationEvent["type"],
  invocationId: string,
  extra: Record<string, unknown> = {},
): ToolInvocationEvent {
  return { type, invocation_id: invocationId, run_id: "run-1", ...extra } as unknown as ToolInvocationEvent;
}

describe("DurableToolTrail (spec: chat-tool-trail)", () => {
  it("renders nothing for a turn without activity", () => {
    const { container } = render(<DurableToolTrail calls={[]} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("shows one collapsed summary by default — no per-call rows", () => {
    render(trail(durable(1), durable(2), durable(3)));
    expect(screen.getByText("3 tools used")).toBeInTheDocument();
    expect(screen.queryByText("mcp_tenant_ida_tool_1")).not.toBeInTheDocument();
  });

  it("expands into one verbatim wire-name row per call", () => {
    render(trail(durable(1), durable(2)));
    fireEvent.click(screen.getByRole("button", { name: /2 tools used/ }));
    expect(screen.getByText("mcp_tenant_ida_tool_1")).toBeInTheDocument();
    expect(screen.getByText("mcp_tenant_ida_tool_2")).toBeInTheDocument();
  });

  it("holds tool rows only — prose is a transcript row, not a body block", () => {
    // The umbrella used to fold a round's prose in as an inert "thinking block",
    // which is how a piece of the answer came to render as thinking and to
    // vanish when reasoning display was withheld. `MessageList` now closes the
    // umbrella on prose and renders it as a message row, so there is no shape
    // for prose to take in here at all.
    render(trail(durable(1), durable(2)));
    fireEvent.click(screen.getByRole("button", { name: /2 tools used/ }));
    const items = Array.from(screen.getByRole("list").querySelectorAll('[role="listitem"]'));
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent("mcp_tenant_ida_tool_1");
    expect(items[1]).toHaveTextContent("mcp_tenant_ida_tool_2");
  });

  it("surfaces a warning cue for unanswered calls, collapsed and expanded", () => {
    render(trail(durable(1), durable(2, "unanswered")));
    expect(screen.getByText("1 without a recorded result")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /2 tools used/ }));
    expect(screen.getByText("No result recorded")).toBeInTheDocument();
  });

  it("durable rows are static headers — no per-row toggle until detail exists", () => {
    render(trail(durable(1)));
    fireEvent.click(screen.getByRole("button", { name: /1 tool used/ }));
    // The only button is the group header; rows expose no trigger.
    expect(screen.getAllByRole("button")).toHaveLength(1);
  });

  it("opens a detailed row into Parameters and Result panels (spec: chat-tool-call-detail)", () => {
    render(
      trail(
        durable(1, "answered", {
          arguments: '{"sql":"select 1"}',
          result: '{"rows":42}',
        }),
      ),
    );
    fireEvent.click(screen.getByRole("button", { name: /1 tool used/ }));
    fireEvent.click(screen.getByRole("button", { name: /mcp_tenant_ida_tool_1/ }));
    expect(screen.getByText("Parameters")).toBeInTheDocument();
    expect(screen.getByText(/"sql": "select 1"/)).toBeInTheDocument();
    expect(screen.getByText("Result")).toBeInTheDocument();
    expect(screen.getByText(/"rows": 42/)).toBeInTheDocument();
  });

  it("renders hostile payloads as literal text — no markup interpretation", () => {
    const hostile = "<img src=x onerror=alert(1)> **bold** [link](http://evil)";
    const { container } = render(trail(durable(1, "answered", { result: hostile })));
    fireEvent.click(screen.getByRole("button", { name: /1 tool used/ }));
    fireEvent.click(screen.getByRole("button", { name: /mcp_tenant_ida_tool_1/ }));
    expect(screen.getByText(hostile)).toBeInTheDocument();
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("a")).toBeNull();
  });

  it("discloses truncation on capped payloads", () => {
    render(trail(durable(1, "answered", { result: "partial…", result_truncated: true })));
    fireEvent.click(screen.getByRole("button", { name: /1 tool used/ }));
    expect(screen.getByText("Output preview shortened")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /mcp_tenant_ida_tool_1/ }));
    expect(screen.getByText(/Truncated at 32 KiB/)).toBeInTheDocument();
  });
});

describe("step rows (spec: chat-tool-trail, task-shaped steps)", () => {
  const expand = () => fireEvent.click(screen.getByRole("button", { name: /tools? used/ }));

  it("leads each row with what happened, in the order the run did it", () => {
    render(
      trail(
        durable(1, "answered", { step: summary("Listed 2 entries", [["src/features/cards.tsx", false]]) }),
        durable(2, "answered", { step: summary("Read", [["README.md", false]]) }),
      ),
    );
    expand();

    const items = Array.from(screen.getByRole("list").querySelectorAll('[role="listitem"]'));
    expect(items[0]).toHaveTextContent("Listed 2 entries");
    expect(items[1]).toHaveTextContent("Read");
  });

  it("keeps the wire name verbatim beside the label", () => {
    render(trail(durable(1, "answered", { step: summary("Listed 2 entries") })));
    expand();
    // The label says what the call did; the name says which tool did it, and
    // neither answers the other's question.
    expect(screen.getByText("Listed 2 entries")).toBeInTheDocument();
    expect(screen.getByText("mcp_tenant_ida_tool_1")).toBeInTheDocument();
  });

  it("renders one chip per path, and no chip is a link or a button", () => {
    const { container } = render(
      trail(
        durable(1, "answered", {
          step: summary("Listed 3 entries", [
            ["src/a.ts", false],
            ["src/b.ts", false],
            ["src/c.ts", false],
          ]),
        }),
      ),
    );
    expand();

    for (const path of ["src/a.ts", "src/b.ts", "src/c.ts"]) {
      expect(screen.getByText(path)).toBeInTheDocument();
    }
    expect(container.querySelector("a")).toBeNull();
    // The umbrella trigger is still the only button in the card.
    expect(screen.getAllByRole("button")).toHaveLength(1);
  });

  it("clicking a chip opens, navigates and fetches nothing", () => {
    render(trail(durable(1, "answered", { step: summary("Read", [["README.md", false]]) })));
    expand();
    const chip = screen.getByText("README.md");
    const open = vi.fn();
    const fetchSpy = vi.fn();
    vi.stubGlobal("open", open);
    vi.stubGlobal("fetch", fetchSpy);

    fireEvent.click(chip);

    expect(open).not.toHaveBeenCalled();
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("keeps a hostile path literal — markdown and HTML never become markup", () => {
    const hostile = "<img src=x onerror=alert(1)>**bold**[l](http://evil).ts";
    const { container } = render(trail(durable(1, "answered", { step: summary("Read", [[hostile, false]]) })));
    expand();

    expect(screen.getByText(hostile)).toBeInTheDocument();
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("strong")).toBeNull();
    expect(container.querySelector("a")).toBeNull();
  });

  it("marks a sensitive path visibly and in its accessible name", () => {
    const { container } = render(
      trail(
        durable(1, "answered", {
          step: summary("Listed 2 entries", [
            ["src/cards.tsx", false],
            [".ssh/id_ed25519", true],
          ]),
        }),
      ),
    );
    expand();

    // Not colour alone: the accessible name says which, and a glyph shows it.
    expect(screen.getByText("Sensitive file: .ssh/id_ed25519")).toBeInTheDocument();
    expect(screen.queryByText("Sensitive file: src/cards.tsx")).not.toBeInTheDocument();
    expect(container.querySelectorAll("svg.lucide-shield-alert")).toHaveLength(1);
  });

  it("states both numbers when it shows fewer paths than the run touched", () => {
    const paths: Array<[string, boolean]> = Array.from({ length: 12 }, (_, index) => [`src/entry-${index}.ts`, false]);
    render(trail(durable(1, "answered", { step: summary("Listed 400 entries", paths, 400) })));
    expand();

    expect(screen.getByText("Listed 400 entries")).toBeInTheDocument();
    expect(screen.getByText("12 of 400 paths")).toBeInTheDocument();
    expect(screen.getByText("src/entry-0.ts")).toBeInTheDocument();
  });

  it("says nothing about a bound when nothing was cut", () => {
    render(
      trail(
        durable(1, "answered", {
          step: summary("Listed 2 entries", [
            ["src/a.ts", false],
            ["src/b.ts", false],
          ]),
        }),
      ),
    );
    expand();
    expect(screen.queryByText(/of \d+ paths/)).not.toBeInTheDocument();
  });

  it("shows a shortened path while keeping the full one reachable", async () => {
    const long = `src/features/${"nested/".repeat(8)}MessageList.tsx`;
    render(trail(durable(1, "answered", { step: summary("Read", [[long, false]]) })));
    expand();

    // Shortened for the row, never shortened in a way that changes which file
    // the chip names: the full value stays the accessible name, and hovering
    // the chip says it in full.
    const spoken = screen.getByText(long, { selector: ".sr-only" });
    const chip = spoken.parentElement as HTMLElement;
    const shown = chip.querySelector("span[aria-hidden]")?.textContent ?? "";
    expect(shown).not.toBe(long);
    expect(shown).toContain("…");
    expect(shown.endsWith("MessageList.tsx")).toBe(true);
    expect(chip).not.toHaveAttribute("title");

    fireEvent.mouseEnter(chip);
    await waitFor(() => expect(screen.getByRole("tooltip")).toHaveTextContent(long));
  });

  it("leaves a summary-less call exactly as it was", () => {
    render(trail(durable(1), durable(2, "answered", { step: summary("Read", [["a.ts", false]]) })));
    expand();

    const items = Array.from(screen.getByRole("list").querySelectorAll('[role="listitem"]'));
    // An MCP row keeps its glyph and its name, with no label and no chips.
    expect(items[0]).toHaveTextContent("mcp_tenant_ida_tool_1");
    expect(items[0].textContent).not.toContain("Read");
    expect(items[1]).toHaveTextContent("Read");
  });

  it("still surfaces the unanswered warning on a row with no outcome to name", () => {
    render(trail(durable(1, "unanswered")));
    expand();
    expect(screen.getByText("No result recorded")).toBeInTheDocument();
  });

  it("reports a settle when a chip-bearing umbrella collapses", async () => {
    // Chips wrap, so a step's height depends on the available width in a way a
    // fixed estimate cannot predict; an unmeasured collapse leaves a gap.
    const paths: Array<[string, boolean]> = Array.from({ length: 12 }, (_, index) => [`src/entry-${index}.ts`, false]);
    const reports: HTMLElement[] = [];
    const view = render(
      <div data-index="2" data-testid="chip-row">
        {trail(durable(1, "answered", { step: summary("Listed 400 entries", paths, 400) }))}
      </div>,
    );
    view.container.addEventListener(REMEASURE_EVENT, (event) => {
      reports.push(event.target as HTMLElement);
    });

    fireEvent.click(screen.getByRole("button", { name: /1 tool used/ }));
    await act(async () => {});
    reports.length = 0;
    fireEvent.click(screen.getByRole("button", { name: /1 tool used/ }));
    await act(async () => {});

    expect(reports.length).toBeGreaterThan(0);
    expect(reports.every((target) => target.closest("[data-index]") === screen.getByTestId("chip-row"))).toBe(true);
  });
});

/**
 * What jsdom can and cannot prove here. It runs NO CSS animations and lays
 * nothing out: every element measures 0, and `getComputedStyle(node)
 * .animationName` is "none" for everything — which is precisely the
 * reduced-motion condition the reporter has to survive, so the motionless
 * path is the one that runs by default and the ANIMATED path is the one that
 * has to be staged — a stubbed `animationName`, then a synthetic
 * `animationend`. These cases therefore prove that a settle report is
 * DISPATCHED, from the right node, at the right moment relative to the
 * animation, and that it is attributed to the row that contains the
 * disclosure. They cannot prove the visual gap closes: there is no height
 * here to shrink. That a report shrinks a row is MessageList's half of the
 * contract and is tested there; that the gap is gone on screen is task 5.3's
 * live pass with the OS setting on.
 */
describe("disclosure settle reports (spec: chat-motion-system)", () => {
  /** Mirrors MessageList: the disclosure sits inside a `[data-index]` row and
   * the listener sits above it, so a report that does not bubble out of the
   * row never arrives — and each one is attributed by the same
   * `closest("[data-index]")` MessageList uses. */
  function renderInRow(ui: ReactNode) {
    const reports: Array<{ row: HTMLElement | null; target: HTMLElement }> = [];
    const view = render(
      <div data-index="3" data-testid="row">
        {ui}
      </div>,
    );
    view.container.addEventListener(REMEASURE_EVENT, (event) => {
      const target = event.target as HTMLElement;
      reports.push({ row: target.closest<HTMLElement>("[data-index]"), target });
    });
    return { ...view, reports, row: screen.getByTestId("row") };
  }

  /** The reporter answers on a microtask (the mutation observer's), so a
   * settle is never read mid-commit. */
  const settle = () => act(async () => {});

  const umbrella = (container: HTMLElement) =>
    container.querySelector<HTMLElement>(".ai-collapsible-content") as HTMLElement;

  /** The one thing jsdom cannot supply: a declared animation. It has to be
   * answered at `getComputedStyle`, not staged as an inline style — Radix owns
   * `node.style.animationName` (it blanks the property to measure the body,
   * then restores the value the node had at mount) and would wipe it. In the
   * app the declaration comes from the stylesheet, which is where this stub
   * puts it. */
  const declareAnimation = (node: HTMLElement, name = "xf-collapse-open") => {
    const real = window.getComputedStyle.bind(window);
    vi.spyOn(window, "getComputedStyle").mockImplementation((element, pseudo) => {
      const styles = real(element, pseudo ?? undefined);
      if (element !== node) return styles;
      return new Proxy(styles, {
        get: (target, property) => (property === "animationName" ? name : Reflect.get(target, property)),
      });
    });
    expect(getComputedStyle(node).animationName).toBe(name);
  };

  it("waits for the animation when one will run — no intermediate frame is reported", async () => {
    const { container, reports, row } = renderInRow(trail(durable(1)));
    declareAnimation(umbrella(container));

    fireEvent.click(screen.getByRole("button", { name: /1 tool used/ }));
    await settle();
    expect(reports).toHaveLength(0);

    fireEvent.animationEnd(umbrella(container));
    expect(reports.map((report) => report.row)).toEqual([row]);
  });

  it("reports an expand that did NOT animate — the reduced-motion path", async () => {
    const { reports, row } = renderInRow(trail(durable(1)));

    fireEvent.click(screen.getByRole("button", { name: /1 tool used/ }));
    await settle();

    // No animationend is ever fired: under `animation: none !important` there
    // is none to fire, and the report has to arrive anyway.
    expect(reports.length).toBeGreaterThan(0);
    expect(reports.every((report) => report.row === row)).toBe(true);
  });

  it("reports a collapse that did NOT animate — the gap this fixes", async () => {
    const { reports, row } = renderInRow(trail(durable(1)));
    fireEvent.click(screen.getByRole("button", { name: /1 tool used/ }));
    await settle();
    reports.length = 0;

    fireEvent.click(screen.getByRole("button", { name: /1 tool used/ }));
    await settle();

    expect(reports.length).toBeGreaterThan(0);
    expect(reports.every((report) => report.row === row)).toBe(true);
  });

  it("reports a nested detail panel's expand and collapse, attributed to its row", async () => {
    const { reports, row } = renderInRow(trail(durable(1, "answered", { arguments: '{"sql":"select 1"}' })));
    fireEvent.click(screen.getByRole("button", { name: /1 tool used/ }));
    await settle();
    reports.length = 0;

    fireEvent.click(screen.getByRole("button", { name: /mcp_tenant_ida_tool_1/ }));
    await settle();
    expect(reports.length).toBeGreaterThan(0);
    expect(reports.every((report) => report.row === row)).toBe(true);

    reports.length = 0;
    fireEvent.click(screen.getByRole("button", { name: /mcp_tenant_ida_tool_1/ }));
    await settle();
    expect(reports.length).toBeGreaterThan(0);
    expect(reports.every((report) => report.row === row)).toBe(true);
  });

  it("still reports when the disclosure body is REMOVED rather than emptied", async () => {
    const { reports, row } = renderInRow(trail(durable(1, "answered", { arguments: '{"sql":"select 1"}' })));
    fireEvent.click(screen.getByRole("button", { name: /1 tool used/ }));
    await settle();
    fireEvent.click(screen.getByRole("button", { name: /mcp_tenant_ida_tool_1/ }));
    await settle();
    reports.length = 0;

    // Collapsing the umbrella takes the nested panel's node out of the
    // document entirely; it cannot dispatch for itself.
    fireEvent.click(screen.getByRole("button", { name: /1 tool used/ }));
    await settle();

    expect(reports.some((report) => report.target === row)).toBe(true);
    expect(reports.every((report) => report.row === row)).toBe(true);
  });

  it("asks the DOM whether anything animates, never the media query", async () => {
    // jsdom ships no `matchMedia`, so a reporter that read the preference
    // would crash rather than report — the spy states the rule instead of
    // relying on that accident.
    const matchMedia = vi.fn(() => ({ matches: false }));
    vi.stubGlobal("matchMedia", matchMedia);
    const { container } = renderInRow(trail(durable(1, "answered", { arguments: '{"sql":"select 1"}' })));

    fireEvent.click(screen.getByRole("button", { name: /1 tool used/ }));
    await settle();
    fireEvent.click(screen.getByRole("button", { name: /mcp_tenant_ida_tool_1/ }));
    await settle();
    fireEvent.animationEnd(umbrella(container));
    fireEvent.click(screen.getByRole("button", { name: /1 tool used/ }));
    await settle();

    // `prefers-reduced-motion` is one of several reasons an animation does not
    // run; a reporter that reads it answers a different question and leaves
    // the others broken.
    expect(matchMedia).not.toHaveBeenCalled();
  });
});

describe("LiveRunActivity (spec: chat-run-activity)", () => {
  const runningEvents = [
    invocationEvent("requested", "inv-1", { tool_id: "mcp.tenant-ida.ida_query" }),
    invocationEvent("started", "inv-1"),
  ];

  const openCard = () => fireEvent.click(screen.getByRole("button", { name: /Thinking/ }));

  it("stays closed during the run, and shows parsed live names once opened", () => {
    render(<LiveRunActivity events={runningEvents} runActive />);
    const disclosure = screen.getByRole("button", { name: /Thinking/ });
    expect(disclosure).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("ida_query")).not.toBeInTheDocument();

    fireEvent.click(disclosure);
    expect(disclosure).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("ida_query")).toBeInTheDocument();
    expect(screen.getByText("· tenant-ida")).toBeInTheDocument();
  });

  it("draws no card at all when the run has requested nothing", () => {
    // A card whose body would be empty is a bordered empty box, and it used to
    // be exactly what an active reasoning-only run drew — auto-expanded, in
    // place of the thinking indicator (spec: chat-run-activity, "A live surface
    // never draws an empty card").
    const { container } = render(<LiveRunActivity events={[]} runActive />);
    expect(container).toBeEmptyDOMElement();
  });

  it("holds tool rows only — the answer is a transcript row, not a card block", () => {
    // The answer used to stream in here as an inert secondary-ink block and then
    // re-render as markdown prose in the transcript when the run settled. There
    // is no prop for it any more; the timeline renders it where its durable form
    // lives (spec: chat-run-activity).
    render(<LiveRunActivity events={runningEvents} runActive />);
    openCard();
    const items = Array.from(screen.getByRole("list").querySelectorAll('[role="listitem"]'));
    expect(items).toHaveLength(1);
    expect(items[0]).toHaveTextContent("ida_query");
  });

  it("is still closed after more of the run arrives", () => {
    const { rerender } = render(<LiveRunActivity events={runningEvents} runActive />);
    const more = [...runningEvents, invocationEvent("completed", "inv-1", { truncated: false })];
    rerender(<LiveRunActivity events={more} runActive />);
    expect(screen.getByRole("button", { name: /Thinking/ })).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("ida_query")).not.toBeInTheDocument();
  });

  it("settles to its tool-count label, still closed", () => {
    const done = [...runningEvents, invocationEvent("completed", "inv-1", { truncated: false })];
    const { rerender } = render(<LiveRunActivity events={done} runActive />);
    rerender(<LiveRunActivity events={done} runActive={false} />);
    expect(screen.queryByText("ida_query")).not.toBeInTheDocument();
    expect(screen.getByText("1 tool used")).toBeInTheDocument();
  });

  it("a card the reader opened stays open as the run continues", () => {
    const { rerender } = render(<LiveRunActivity events={runningEvents} runActive />);
    openCard();
    rerender(<LiveRunActivity events={[...runningEvents]} runActive />);
    expect(screen.getByText("ida_query")).toBeInTheDocument();
  });

  it("failed runs label the header and expose the safe error in the row detail", () => {
    const failed = [
      invocationEvent("requested", "inv-1", { tool_id: "mcp.tenant-ida.ida_query" }),
      invocationEvent("failed", "inv-1", { message: "backend said no", retryable: false }),
    ];
    render(<LiveRunActivity events={failed} runActive={false} />);
    expect(screen.getByText("Tool run failed")).toBeInTheDocument();
    // Failed rows are the one live-row type with a disclosure (the error).
    fireEvent.click(screen.getByRole("button", { name: /Tool run failed/ }));
    fireEvent.click(screen.getByRole("button", { name: /ida_query/ }));
    expect(screen.getByRole("alert")).toHaveTextContent("backend said no");
  });

  it("renders the complete lifecycle vocabulary, including a shortened success", () => {
    const events = [
      invocationEvent("requested", "pending", { tool_id: "mcp.warehouse.pending" }),
      invocationEvent("approval_required", "approval", { tool_id: "mcp.warehouse.approval" }),
      invocationEvent("requested", "running", { tool_id: "mcp.warehouse.running" }),
      invocationEvent("started", "running"),
      invocationEvent("requested", "complete", { tool_id: "mcp.warehouse.complete" }),
      invocationEvent("completed", "complete", { truncated: true }),
      invocationEvent("requested", "failed", { tool_id: "mcp.warehouse.failed" }),
      invocationEvent("failed", "failed", { message: "safe failure", retryable: false }),
      invocationEvent("requested", "denied", { tool_id: "mcp.warehouse.denied" }),
      invocationEvent("denied", "denied", { reason_code: "user_declined" }),
      invocationEvent("requested", "cancelled", { tool_id: "mcp.warehouse.cancelled" }),
      invocationEvent("cancelled", "cancelled"),
    ];

    const { container } = render(<LiveRunActivity events={events} runActive />);
    fireEvent.click(screen.getByRole("button", { name: /Thinking/ }));

    for (const label of ["Pending", "Awaiting approval", "Running", "Failed", "Denied", "Cancelled"]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
    expect(screen.getByText("Output shortened")).toBeInTheDocument();
    expect(container.querySelector('[data-state="output-available"]')).not.toBeNull();
    expect(screen.queryByText("Completed")).not.toBeInTheDocument();
  });
});

describe("Unified Activity (spec: unified activity pill)", () => {
  it("DurableToolTrail combines reasoning and tools in one disclosure", () => {
    render(
      <DurableToolTrail
        asides={[{ kind: "reasoning", text: "I need to inspect the data first.", afterCallCount: 0 }]}
        calls={[durable(1), durable(2)]}
      />,
    );
    expect(screen.getByText("Thought process · 2 tools used")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Thought process · 2 tools used/ }));
    expect(screen.getByText("Reasoning")).toBeInTheDocument();
    expect(screen.getByText("I need to inspect the data first.")).toBeInTheDocument();
    expect(screen.getByText("mcp_tenant_ida_tool_1")).toBeInTheDocument();
    expect(screen.getByText("mcp_tenant_ida_tool_2")).toBeInTheDocument();
  });

  it("LiveRunActivity combines reasoning and tool events in one active disclosure", () => {
    const runningEvents = [invocationEvent("requested", "inv-1", { tool_id: "mcp.tenant-ida.ida_query" })];
    render(
      <LiveRunActivity
        asides={[{ kind: "reasoning", text: "Analyzing the query structure.", afterCallCount: 0 }]}
        events={runningEvents}
        runActive
      />,
    );
    expect(screen.getByText(/Thinking/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Thinking/ }));
    expect(screen.getByText("Analyzing the query structure.")).toBeInTheDocument();
    expect(screen.getByText("ida_query")).toBeInTheDocument();
  });

  it("LiveRunActivity shows Thought process · X tool calls when settled with reasoning", () => {
    const doneEvents = [
      invocationEvent("requested", "inv-1", { tool_id: "mcp.tenant-ida.ida_query" }),
      invocationEvent("completed", "inv-1", { truncated: false }),
    ];
    render(
      <LiveRunActivity
        asides={[{ kind: "reasoning", text: "Done with query analysis.", afterCallCount: 0 }]}
        events={doneEvents}
        runActive={false}
      />,
    );
    expect(screen.getByText("Thought process · 1 tool used")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Thought process · 1 tool used/ }));
    expect(screen.getByText("Done with query analysis.")).toBeInTheDocument();
    expect(screen.getByText("ida_query")).toBeInTheDocument();
  });
});
