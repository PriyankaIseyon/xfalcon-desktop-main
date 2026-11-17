import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import type { ChatToolCallView, ToolInvocationEvent } from "../../lib/generated/contracts";
import { DEFAULT_APPEARANCE, applyAppearance, chooseAppearance, resetAppearanceForTests } from "../../app/appearance";
import { LiveRunTimeline } from "./LiveRunTimeline";
import { MessageList } from "./MessageList";
import { DurableToolTrail, LiveRunActivity } from "./ToolActivityCard";
import type { ChatMessageRow } from "./chatStore";

/**
 * What the SHIPPED surface draws, with the real `./activityDisclosure` flags and
 * the real appearance default — deliberately unmocked here, unlike the suites
 * that force the flags on to keep proving the properties behind them.
 *
 * The point of each assertion is that the withheld content was PRESENT in the
 * props and still did not reach the DOM. A test that simply passed nothing would
 * pass whether the flag worked or not.
 *
 * Reasoning is a preference, hidden by default
 * (`appearance.show_reasoning`), so both of its states are
 * pinned: shown, it is reachable inside the run's unified disclosure; hidden,
 * no surface draws a card for it and none draws an empty one either.
 */

const REASONING = "I should check the column names before I trust them.";
const ARGUMENTS = '{"table":"FACT_CUSTOMER_PERFORMANCE","sql":"SELECT SUM(NET_SALES)"}';
const RESULT = '{"rows":132650,"net_sales":75600000}';

function call(overrides: Partial<ChatToolCallView> = {}): ChatToolCallView {
  return {
    version: 1,
    call_id: "call-1",
    name: "ida_estimate_rows",
    status: "succeeded",
    arguments: ARGUMENTS,
    result: RESULT,
    arguments_truncated: false,
    result_truncated: false,
    step: {
      version: 1,
      label: "Estimated rows",
      paths: [
        {
          version: 1,
          display: "FACT_CUSTOMER_PERFORMANCE",
          full: "falcon-cdp-demo.FACT_CUSTOMER_PERFORMANCE",
          sensitive: false,
        },
      ],
      total_paths: 1,
    },
    ...overrides,
  } as ChatToolCallView;
}

function events(): ToolInvocationEvent[] {
  return [
    { type: "requested", version: 1, invocation_id: "inv-1", run_id: "run-1", tool_id: "mcp.tenant.ida_estimate_rows" },
    { type: "failed", version: 1, invocation_id: "inv-1", run_id: "run-1", message: RESULT },
  ] as ToolInvocationEvent[];
}

function message(overrides: Partial<ChatMessageRow> & { id: string }): ChatMessageRow {
  return {
    role: "assistant",
    text: "",
    sequence: 1n,
    toolCalls: [],
    ...overrides,
  } as ChatMessageRow;
}

function timeline(overrides: Partial<Parameters<typeof LiveRunTimeline>[0]> = {}) {
  return <LiveRunTimeline activity={[]} reasoning={[]} reasoningTrimmed={false} runActive {...overrides} />;
}

/** Turns the preference off through the same path the settings control uses, so
 * the test exercises the store rather than a mock of it. `applyAppearance`
 * writes to a detached element: the real one belongs to the app, not to a test. */
async function hideReasoning() {
  const target = { dataset: {} as Record<string, string> };
  applyAppearance(DEFAULT_APPEARANCE, target);
  await chooseAppearance({ ...DEFAULT_APPEARANCE, show_reasoning: false }, target).catch(() => {
    // No Tauri host in jsdom, so the persist leg rejects. The applied value is
    // what this test reads, and that is committed before the write.
  });
}

async function showReasoning() {
  // Exercise the real appearance store. No native host is available in jsdom;
  // the choice is applied before the persistence leg rejects.
  await chooseAppearance({ ...DEFAULT_APPEARANCE, show_reasoning: true }, { dataset: {} }).catch(() => {});
}

afterEach(cleanup);
afterEach(resetAppearanceForTests);

describe("the shipped activity disclosure", () => {
  it("names the tools a settled run called, and nothing about how", async () => {
    const user = userEvent.setup();
    render(<DurableToolTrail calls={[call()]} />);

    await user.click(screen.getByRole("button", { name: /1 tool used/ }));

    // The tool's own name is the whole row.
    expect(screen.getByText("ida_estimate_rows")).toBeInTheDocument();
    // Its arguments, its result, and the objects it touched were all handed to
    // the card and none of them are drawn.
    expect(screen.queryByText(/FACT_CUSTOMER_PERFORMANCE/)).not.toBeInTheDocument();
    expect(screen.queryByText(/SELECT SUM/)).not.toBeInTheDocument();
    expect(screen.queryByText(/132650/)).not.toBeInTheDocument();
    expect(screen.queryByText("Estimated rows")).not.toBeInTheDocument();
    expect(screen.queryByText(/Parameters/)).not.toBeInTheDocument();
    expect(screen.queryByText(/Result/)).not.toBeInTheDocument();
  });

  it("keeps a failed call visibly failed while withholding why", async () => {
    const user = userEvent.setup();
    render(<LiveRunActivity events={events()} runActive={false} />);

    // The header still says the run failed, so the failure is not hidden.
    const header = screen.getByRole("button", { name: /Tool run failed/ });
    await user.click(header);
    expect(screen.getByText("ida_estimate_rows")).toBeInTheDocument();
    // The error text quotes what came back from the tool, so it stays withheld.
    expect(screen.queryByText(/132650/)).not.toBeInTheDocument();
  });

  it("offers a run's reasoning inside its unified collapsed disclosure", async () => {
    await showReasoning();
    const user = userEvent.setup();
    render(timeline({ reasoning: [{ text: REASONING, afterCallCount: 0 }] }));

    // Collapsed: the run says it thought, without taking the reading surface.
    expect(screen.queryByText(REASONING)).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Thinking/ }));
    expect(screen.getByText(REASONING)).toBeInTheDocument();
  });

  it("draws no card and no empty box when reasoning is hidden", async () => {
    await hideReasoning();
    const { container } = render(timeline({ reasoning: [{ text: REASONING, afterCallCount: 0 }] }));

    // The defect this pins: with reasoning withheld, an active run that had only
    // reasoned drew an auto-expanded bordered card whose every block rendered
    // null, in place of its thinking indicator.
    expect(container).toBeEmptyDOMElement();
    expect(screen.queryByText(REASONING)).not.toBeInTheDocument();
  });

  it("leaves a hidden run's reasoning no umbrella in the transcript either", async () => {
    await hideReasoning();
    render(
      <MessageList
        messages={[message({ id: "m-1", text: "Here is the answer." })]}
        reasoning={[{ text: REASONING, afterCallCount: 0 }]}
      />,
    );
    expect(screen.queryByRole("button", { name: /Thought process/ })).not.toBeInTheDocument();
    expect(screen.queryByText(REASONING)).not.toBeInTheDocument();
    expect(screen.getByText("Here is the answer.")).toBeInTheDocument();
  });

  it("draws a settled run's reasoning in the transcript when it is shown", async () => {
    await showReasoning();
    const user = userEvent.setup();
    render(
      <MessageList
        messages={[message({ id: "m-1", text: "Here is the answer." })]}
        reasoning={[{ text: REASONING, afterCallCount: 0 }]}
      />,
    );
    await user.click(screen.getByRole("button", { name: /Thought process/ }));
    expect(screen.getByText(REASONING)).toBeInTheDocument();
    expect(screen.getByText("Here is the answer.")).toBeInTheDocument();
  });
});
