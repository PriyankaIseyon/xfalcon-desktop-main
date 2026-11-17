import { act, cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ToolInvocationEvent } from "../../lib/generated/contracts";
import { ChatSurface } from "./ChatSurface";
import type { ChatMessageRow } from "./chatStore";

afterEach(cleanup);
afterEach(() => vi.unstubAllGlobals());

/** Animation frames the test drives by hand, on a clock it owns. */
let frames = new Map<number, FrameRequestCallback>();
let clock = 0;
let nextFrameId = 1;
function stubFrames(reducedMotion: boolean) {
  frames = new Map();
  clock = 0;
  nextFrameId = 1;
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    frames.set(nextFrameId, callback);
    return nextFrameId++;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
  vi.stubGlobal(
    "matchMedia",
    vi.fn(() => ({ matches: reducedMotion })),
  );
}
function flushFrames(count: number) {
  for (let index = 0; index < count && frames.size > 0; index += 1) {
    clock += 1000 / 60;
    const pending = [...frames.values()];
    frames.clear();
    act(() => {
      for (const callback of pending) callback(clock);
    });
  }
}

const ANSWER = "The answer, streamed once and never drawn a second time.";

const user: ChatMessageRow = { id: "m-1", role: "user", text: "hi", sequence: BigInt(1), toolCalls: [] };
const saved = (text = ANSWER): ChatMessageRow => ({
  id: "m-3",
  role: "assistant",
  text,
  sequence: BigInt(3),
  toolCalls: [],
});
const toolRound: ChatMessageRow = {
  id: "m-2",
  role: "assistant",
  text: "",
  sequence: BigInt(2),
  toolCalls: [
    {
      call_id: "call-1",
      name: "mcp_tool_1",
      status: "answered",
      arguments: null,
      arguments_truncated: false,
      result: null,
      result_truncated: false,
      step: null,
    },
  ],
};
const requested = {
  type: "requested",
  version: 1,
  invocation_id: "inv-1",
  run_id: "run-1",
  tool_id: "mcp.tenant.ida_query",
} as unknown as ToolInvocationEvent;

type Phase = {
  name: string;
  props: Partial<Parameters<typeof ChatSurface>[0]>;
};

/** The props ChatPane hands the surface at each step of one answer's life. */
function phases(withTools: boolean): Phase[] {
  const activity = withTools ? [requested] : [];
  const history = [user];
  const settled = withTools ? [user, toolRound, saved()] : [user, saved()];
  return [
    {
      name: "streaming",
      props: {
        messages: history,
        activity,
        liveText: ANSWER,
        answerStreaming: true,
        streamRunId: "run-1",
        runActive: true,
      },
    },
    {
      name: "provider finished, run not yet settled",
      props: {
        messages: history,
        activity,
        liveText: ANSWER,
        answerStreaming: false,
        streamRunId: "run-1",
        runActive: true,
      },
    },
    {
      name: "run settled, saved row not yet landed",
      props: {
        messages: history,
        activity,
        liveText: ANSWER,
        answerStreaming: false,
        streamRunId: null,
        runActive: false,
      },
    },
    {
      name: "saved row landed",
      props: {
        messages: settled,
        activity: [],
        liveText: "",
        answerStreaming: false,
        streamRunId: null,
        runActive: false,
      },
    },
  ];
}

describe.each([
  ["a run with no tools", false],
  ["a run that used a tool", true],
])("the answer of %s", (_label, withTools) => {
  it.each([
    ["with the typewriter reveal", false],
    ["with reduced motion", true],
  ])("is one piece of DOM from its first token to its saved row, %s", (_motion, reducedMotion) => {
    stubFrames(reducedMotion);
    const [first, ...rest] = phases(withTools);
    const { container, rerender } = render(<ChatSurface {...first.props} />);
    flushFrames(3);
    const answer = container.querySelector(".assistant-markdown");
    expect(answer).not.toBeNull();

    for (const phase of rest) {
      rerender(<ChatSurface {...phase.props} />);
      flushFrames(3);
      const current = container.querySelectorAll(".assistant-markdown");
      expect(current, phase.name).toHaveLength(1);
      expect(current[0], `${phase.name}: the answer was rebuilt`).toBe(answer);
      expect(answer?.isConnected, phase.name).toBe(true);
      // Mid-reveal it shows a prefix of the answer, never anything else.
      expect(ANSWER.startsWith(answer?.textContent ?? "x"), phase.name).toBe(true);
    }
    flushFrames(300);
    expect(answer?.textContent).toBe(ANSWER);
    expect(container.querySelector(".streaming-caret")).toBeNull();
  });
});
